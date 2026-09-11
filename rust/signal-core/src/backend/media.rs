// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::codecs::jpeg::JpegEncoder;
use image::{ImageFormat, ImageReader};
use presage::proto::{AttachmentPointer, attachment_pointer};
use sha2::{Digest, Sha256};

pub const MAX_INLINE_MEDIA_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_INLINE_GIF_EDGE: usize = 8192;
pub const MAX_INLINE_GIF_PIXELS: usize = 16 * 1000 * 1000;
pub const MAX_INLINE_GIF_PIXEL_FRAMES: usize = 8 * 1000 * 1000;
pub const MAX_SIGNAL_GIF_TRANSCODES_PER_MESSAGE: usize = 2;
pub const SIGNAL_GIF_TRANSCODE_POLL_INTERVAL: Duration = Duration::from_millis(20);
pub const SIGNAL_GIF_TRANSCODE_TIMEOUT: Duration = Duration::from_secs(15);
pub const SIGNAL_GIF_FFMPEG: &str = "/usr/bin/ffmpeg";
pub const SIGNAL_GIF_PRLIMIT: &str = "/usr/bin/prlimit";
pub const SIGNAL_GIF_ADDRESS_SPACE_LIMIT: &str = "--as=1073741824:1073741824";

static SIGNAL_GIF_TRANSCODE_LOCK: Mutex<()> = Mutex::new(());

const MAX_AVATAR_DIMENSION: u32 = 192;
const AVATAR_JPEG_QUALITY: u8 = 85;

pub fn downscale_avatar(raw_data: &[u8]) -> Result<Vec<u8>, String> {
    let reader = ImageReader::new(Cursor::new(raw_data))
        .with_guessed_format()
        .map_err(|e| format!("unrecognized image format: {e}"))?;

    let format = reader.format();

    // Fast-path: read image dimensions from header
    if let Ok(dimensions) = reader.into_dimensions()
        && dimensions.0 <= MAX_AVATAR_DIMENSION
        && dimensions.1 <= MAX_AVATAR_DIMENSION
    {
        return Ok(raw_data.to_vec());
    }

    let dynamic_img =
        image::load_from_memory(raw_data).map_err(|e| format!("image decode error: {e}"))?;

    let resized = dynamic_img.thumbnail(MAX_AVATAR_DIMENSION, MAX_AVATAR_DIMENSION);

    let mut output = Vec::new();
    if format == Some(ImageFormat::Png) || resized.color().has_alpha() {
        resized
            .write_to(&mut Cursor::new(&mut output), ImageFormat::Png)
            .map_err(|e| format!("png encode error: {e}"))?;
    } else {
        let mut encoder = JpegEncoder::new_with_quality(&mut output, AVATAR_JPEG_QUALITY);
        encoder
            .encode_image(&resized)
            .map_err(|e| format!("jpeg encode error: {e}"))?;
    }

    Ok(output)
}

pub type CachedAvatar = (Vec<u8>, String);
pub type AvatarMemoryCache = Arc<Mutex<HashMap<[u8; 32], CachedAvatar>>>;

#[derive(Clone)]
pub struct AvatarCache {
    cache_dir: Option<PathBuf>,
    mem_cache: AvatarMemoryCache,
}

impl AvatarCache {
    pub fn new(store_path: Option<&str>) -> Self {
        let cache_dir = store_path.and_then(|path| {
            let parent = Path::new(path).parent()?;
            let dir = parent.join("avatar_cache");
            if std::fs::create_dir_all(&dir).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
                }
                Some(dir)
            } else {
                None
            }
        });
        Self {
            cache_dir,
            mem_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn prepare_avatar(&self, raw_data: Vec<u8>) -> (Vec<u8>, String) {
        if raw_data.is_empty() {
            return (raw_data, String::new());
        }

        let raw_hash: [u8; 32] = Sha256::digest(&raw_data).into();

        // 1. L1 Memory Cache Check
        if let Ok(guard) = self.mem_cache.lock()
            && let Some(hit) = guard.get(&raw_hash)
        {
            return hit.clone();
        }

        let raw_hash_hex = hex::encode(raw_hash);

        // 2. L2 Disk Cache Check
        if let Some(ref cache_dir) = self.cache_dir {
            let disk_path = cache_dir.join(&raw_hash_hex);
            if let Ok(disk_bytes) = std::fs::read(&disk_path) {
                let checksum = hex::encode(Sha256::digest(&disk_bytes));
                if let Ok(mut guard) = self.mem_cache.lock() {
                    guard.insert(raw_hash, (disk_bytes.clone(), checksum.clone()));
                }
                return (disk_bytes, checksum);
            }
        }

        // 3. Downscale or Fallback
        let downscaled = match downscale_avatar(&raw_data) {
            Ok(processed) => processed,
            Err(err) => {
                tracing::warn!("Avatar downscale skipped: {err}; using raw payload");
                raw_data
            }
        };

        let checksum = hex::encode(Sha256::digest(&downscaled));

        // 4. Atomic Disk Persistence
        if let Some(ref cache_dir) = self.cache_dir {
            let disk_path = cache_dir.join(&raw_hash_hex);
            let tmp_path = cache_dir.join(format!("{raw_hash_hex}.tmp.{}", std::process::id()));
            if std::fs::write(&tmp_path, &downscaled).is_ok() {
                let _ = std::fs::rename(&tmp_path, &disk_path);
            }
        }

        // 5. Store in L1 Memory Cache
        if let Ok(mut guard) = self.mem_cache.lock() {
            guard.insert(raw_hash, (downscaled.clone(), checksum.clone()));
        }

        (downscaled, checksum)
    }
}

pub fn inline_image_matches(content_type: Option<&str>, data: &[u8]) -> bool {
    if data.len() > MAX_INLINE_MEDIA_BYTES {
        return false;
    }
    match content_type {
        Some(content_type) if content_type.eq_ignore_ascii_case("image/jpeg") => {
            data.starts_with(&[0xff, 0xd8, 0xff])
        }
        Some(content_type) if content_type.eq_ignore_ascii_case("image/png") => {
            data.starts_with(b"\x89PNG\r\n\x1a\n")
        }
        Some(content_type) if content_type.eq_ignore_ascii_case("image/gif") => {
            data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")
        }
        _ => false,
    }
}

pub fn should_inline_image(
    outgoing: bool,
    content_type: Option<&str>,
    data: Option<&[u8]>,
) -> bool {
    !outgoing && data.is_some_and(|data| inline_image_matches(content_type, data))
}

pub fn gif_u16(data: &[u8], offset: usize) -> Option<usize> {
    let encoded: [u8; 2] = data.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(encoded) as usize)
}

pub fn advance_gif_offset(offset: &mut usize, amount: usize, size: usize) -> bool {
    if *offset > size || amount > size - *offset {
        return false;
    }
    *offset += amount;
    true
}

pub fn skip_gif_sub_blocks(data: &[u8], offset: &mut usize) -> bool {
    while *offset < data.len() {
        let block_size = data[*offset] as usize;
        *offset += 1;
        if block_size == 0 {
            return true;
        }
        if !advance_gif_offset(offset, block_size, data.len()) {
            return false;
        }
    }
    false
}

pub fn bounded_inline_gif(data: &[u8]) -> bool {
    if data.len() < 13
        || data.len() > MAX_INLINE_MEDIA_BYTES
        || !(data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"))
    {
        return false;
    }

    let Some(canvas_width) = gif_u16(data, 6) else {
        return false;
    };
    let Some(canvas_height) = gif_u16(data, 8) else {
        return false;
    };
    let Some(canvas_pixels) = canvas_width.checked_mul(canvas_height) else {
        return false;
    };
    if canvas_width == 0
        || canvas_height == 0
        || canvas_width > MAX_INLINE_GIF_EDGE
        || canvas_height > MAX_INLINE_GIF_EDGE
        || canvas_pixels > MAX_INLINE_GIF_PIXELS
    {
        return false;
    }

    let mut offset = 13usize;
    let packed = data[10];
    if packed & 0x80 != 0 {
        let color_table_size = 3usize << ((packed & 0x07) as usize + 1);
        if !advance_gif_offset(&mut offset, color_table_size, data.len()) {
            return false;
        }
    }

    let mut frames = 0usize;
    while offset < data.len() {
        let marker = data[offset];
        offset += 1;
        match marker {
            0x3b => return frames > 0,
            0x21 => {
                if !advance_gif_offset(&mut offset, 1, data.len())
                    || !skip_gif_sub_blocks(data, &mut offset)
                {
                    return false;
                }
            }
            0x2c => {
                if offset > data.len() || 9 > data.len() - offset {
                    return false;
                }
                let Some(left) = gif_u16(data, offset) else {
                    return false;
                };
                let Some(top) = gif_u16(data, offset + 2) else {
                    return false;
                };
                let Some(width) = gif_u16(data, offset + 4) else {
                    return false;
                };
                let Some(height) = gif_u16(data, offset + 6) else {
                    return false;
                };
                let image_packed = data[offset + 8];
                offset += 9;

                if width == 0
                    || height == 0
                    || left
                        .checked_add(width)
                        .is_none_or(|right| right > canvas_width)
                    || top
                        .checked_add(height)
                        .is_none_or(|bottom| bottom > canvas_height)
                {
                    return false;
                }
                frames += 1;
                if frames > MAX_INLINE_GIF_PIXEL_FRAMES / canvas_pixels {
                    return false;
                }

                if image_packed & 0x80 != 0 {
                    let color_table_size = 3usize << ((image_packed & 0x07) as usize + 1);
                    if !advance_gif_offset(&mut offset, color_table_size, data.len()) {
                        return false;
                    }
                }
                if offset >= data.len() || !(2..=8).contains(&data[offset]) {
                    return false;
                }
                offset += 1;
                if !skip_gif_sub_blocks(data, &mut offset) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    false
}

pub fn mp4_file_type_box_matches(data: &[u8]) -> bool {
    if data.len() < 16 || data.get(4..8) != Some(b"ftyp") {
        return false;
    }
    let Some(encoded_size) = data.get(..4) else {
        return false;
    };
    let Ok(encoded_size) = <[u8; 4]>::try_from(encoded_size) else {
        return false;
    };
    let box_size = u32::from_be_bytes(encoded_size) as usize;
    (16..=data.len()).contains(&box_size)
}

pub fn signal_gif_video_matches(attachment: &AttachmentPointer, data: &[u8]) -> bool {
    data.len() <= MAX_INLINE_MEDIA_BYTES
        && attachment
            .content_type
            .as_deref()
            .is_some_and(|content_type| content_type.eq_ignore_ascii_case("video/mp4"))
        && attachment.flags.unwrap_or_default() & attachment_pointer::Flags::Gif as u32 != 0
        && mp4_file_type_box_matches(data)
}

pub fn signal_gif_inline_filename(attachment: &AttachmentPointer) -> String {
    let Some(filename) = attachment
        .file_name
        .as_deref()
        .filter(|filename| !filename.is_empty())
    else {
        return "signal-animation.gif".to_owned();
    };
    let basename = filename.rsplit(['/', '\\']).next().unwrap_or_default();
    if basename.is_empty() || basename == "." || basename == ".." {
        return "signal-animation.gif".to_owned();
    }
    let stem = basename
        .rsplit_once('.')
        .and_then(|(stem, _)| (!stem.is_empty()).then_some(stem))
        .unwrap_or(basename);
    format!("{stem}.gif")
}

pub fn attachment_display_name(attachment: &AttachmentPointer) -> &str {
    if let Some(file_name) = attachment
        .file_name
        .as_deref()
        .filter(|file_name| !file_name.is_empty())
    {
        return file_name;
    }

    let content_type = attachment.content_type.as_deref();
    let is_gif = attachment.flags.unwrap_or_default() & attachment_pointer::Flags::Gif as u32 != 0;
    if is_gif {
        if content_type.is_some_and(|value| value.eq_ignore_ascii_case("image/gif")) {
            return "signal-animation.gif";
        }
        if content_type.is_some_and(|value| value.eq_ignore_ascii_case("video/mp4")) {
            return "signal-animation.mp4";
        }
        return "signal-animation";
    }

    match content_type {
        Some(value) if value.eq_ignore_ascii_case("image/jpeg") => "signal-image.jpg",
        Some(value) if value.eq_ignore_ascii_case("image/png") => "signal-image.png",
        Some(value) if value.eq_ignore_ascii_case("image/gif") => "signal-animation.gif",
        Some(value) if value.eq_ignore_ascii_case("video/mp4") => "signal-video.mp4",
        _ => "signal-attachment",
    }
}

pub struct DownloadedAttachment {
    pub attachment_index: usize,
    pub filename: String,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
    pub signal_gif_filename: Option<String>,
}

impl DownloadedAttachment {
    pub fn new(attachment_index: usize, attachment: &AttachmentPointer, data: Vec<u8>) -> Self {
        let signal_gif_filename = signal_gif_video_matches(attachment, &data)
            .then(|| signal_gif_inline_filename(attachment));
        Self {
            attachment_index,
            filename: attachment_display_name(attachment).to_owned(),
            content_type: attachment.content_type.clone(),
            data,
            signal_gif_filename,
        }
    }

    pub fn apply_signal_gif(&mut self, gif: Vec<u8>) -> bool {
        let Some(filename) = self.signal_gif_filename.clone() else {
            return false;
        };
        if !bounded_inline_gif(&gif) {
            return false;
        }

        self.filename = filename;
        self.signal_gif_filename = None;
        self.content_type = Some("image/gif".to_owned());
        self.data = gif;
        true
    }
}

pub fn stop_transcode_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn read_transcode_output(mut output: impl Read) -> Option<Vec<u8>> {
    let mut collected = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = output.read(&mut chunk).ok()?;
        if read == 0 {
            return Some(collected);
        }
        if read > MAX_INLINE_MEDIA_BYTES.saturating_sub(collected.len()) {
            return None;
        }
        collected.extend_from_slice(&chunk[..read]);
    }
}

pub fn signal_gif_transcode_stderr() -> std::process::Stdio {
    #[cfg(test)]
    if std::env::var_os("SIGNAL_PURPLE_REQUIRE_FFMPEG_TEST").is_some() {
        return std::process::Stdio::inherit();
    }
    std::process::Stdio::null()
}

pub fn transcode_signal_gif_video_blocking(input: Vec<u8>) -> Option<Vec<u8>> {
    let _permit = SIGNAL_GIF_TRANSCODE_LOCK.try_lock().ok()?;
    if !Path::new(SIGNAL_GIF_FFMPEG).is_file() || !Path::new(SIGNAL_GIF_PRLIMIT).is_file() {
        return None;
    }

    // Fixed Debian paths and arguments avoid shell or inherited PATH handling.
    // prlimit execs FFmpeg in the same child, so kill and wait cover both.
    let mut child = std::process::Command::new(SIGNAL_GIF_PRLIMIT)
        .args([
            SIGNAL_GIF_ADDRESS_SPACE_LIMIT,
            "--cpu=10:12",
            "--nofile=64:64",
            "--",
            SIGNAL_GIF_FFMPEG,
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-max_alloc",
            "134217728",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-filter_complex_threads",
            "1",
            "-protocol_whitelist",
            "pipe",
            "-probesize",
            "8388608",
            "-analyzeduration",
            "5000000",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            "scale=w='min(480,iw)':h='min(480,ih)':force_original_aspect_ratio=decrease:flags=lanczos",
            "-fpsmax",
            "15",
            "-threads",
            "1",
            "-loop",
            "0",
            "-f",
            "gif",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(signal_gif_transcode_stderr())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C")
        .spawn()
        .ok()?;

    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        stop_transcode_child(&mut child);
        return None;
    };
    let writer = match std::thread::Builder::new()
        .name("signal-gif-input".to_owned())
        .spawn(move || stdin.write_all(&input).is_ok())
    {
        Ok(writer) => writer,
        Err(_) => {
            stop_transcode_child(&mut child);
            return None;
        }
    };
    let reader = match std::thread::Builder::new()
        .name("signal-gif-output".to_owned())
        .spawn(move || read_transcode_output(stdout))
    {
        Ok(reader) => reader,
        Err(_) => {
            stop_transcode_child(&mut child);
            let _ = writer.join();
            return None;
        }
    };

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < SIGNAL_GIF_TRANSCODE_TIMEOUT => {
                std::thread::sleep(SIGNAL_GIF_TRANSCODE_POLL_INTERVAL);
            }
            Ok(None) | Err(_) => {
                stop_transcode_child(&mut child);
                break None;
            }
        }
    };
    let input_complete = writer.join().unwrap_or(false);
    let output = reader.join().ok().flatten();
    if !input_complete || !status.is_some_and(|status| status.success()) {
        return None;
    }
    output.filter(|output| bounded_inline_gif(output))
}

pub async fn transcode_signal_gif_video(input: &[u8]) -> Option<Vec<u8>> {
    let input = input.to_vec();
    tokio::task::spawn_blocking(move || transcode_signal_gif_video_blocking(input))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);

            loop {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("signal-purple-{label}-{}-{id}", std::process::id()));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("could not create test directory: {error}"),
                }
            }
        }

        fn join(&self, path: &str) -> PathBuf {
            self.0.join(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn encoded_gif(width: u16, height: u16, frames: usize) -> Vec<u8> {
        let mut data = b"GIF89a".to_vec();
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(&[0x80, 0x00, 0x00]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0xff, 0xff, 0xff]);
        for _ in 0..frames {
            data.push(0x2c);
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&width.to_le_bytes());
            data.extend_from_slice(&height.to_le_bytes());
            data.push(0x00);
            data.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
        }
        data.push(0x3b);
        data
    }

    fn signal_gif_mp4() -> Vec<u8> {
        let mut data = 24u32.to_be_bytes().to_vec();
        data.extend_from_slice(b"ftypisom");
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"isomiso2");
        data
    }

    #[test]
    fn supplies_useful_names_for_unnamed_media_attachments() {
        let video = AttachmentPointer {
            content_type: Some("video/mp4".into()),
            ..AttachmentPointer::default()
        };
        let animation = AttachmentPointer {
            content_type: Some("video/mp4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };
        let named = AttachmentPointer {
            file_name: Some("shared clip".into()),
            content_type: Some("video/mp4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };

        assert_eq!(attachment_display_name(&video), "signal-video.mp4");
        assert_eq!(attachment_display_name(&animation), "signal-animation.mp4");
        assert_eq!(attachment_display_name(&named), "shared clip");
        assert_eq!(
            attachment_display_name(&AttachmentPointer::default()),
            "signal-attachment"
        );
    }

    #[test]
    fn recognizes_only_bounded_signal_gif_mp4_payloads() {
        let animation = AttachmentPointer {
            content_type: Some("video/mp4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };
        let uppercase = AttachmentPointer {
            content_type: Some("VIDEO/MP4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };
        let video = AttachmentPointer {
            content_type: Some("video/mp4".into()),
            ..AttachmentPointer::default()
        };
        let mp4 = signal_gif_mp4();

        assert!(signal_gif_video_matches(&animation, &mp4));
        assert!(signal_gif_video_matches(&uppercase, &mp4));
        assert!(!signal_gif_video_matches(&video, &mp4));
        assert!(!signal_gif_video_matches(&animation, b"not an mp4"));

        let mut invalid_box = mp4.clone();
        invalid_box[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(!signal_gif_video_matches(&animation, &invalid_box));

        let mut oversized = vec![0u8; MAX_INLINE_MEDIA_BYTES + 1];
        oversized[..mp4.len()].copy_from_slice(&mp4);
        assert!(!signal_gif_video_matches(&animation, &oversized));
    }

    #[test]
    fn validates_generated_gif_structure_and_frame_budget() {
        let gif = encoded_gif(1, 1, 2);
        let excessive_frames = encoded_gif(1000, 1000, 9);
        let mut truncated = gif.clone();
        truncated.pop();
        let mut oversized = gif.clone();
        oversized.resize(MAX_INLINE_MEDIA_BYTES + 1, 0);

        assert!(bounded_inline_gif(&gif));
        assert!(!bounded_inline_gif(&excessive_frames));
        assert!(!bounded_inline_gif(&truncated));
        assert!(!bounded_inline_gif(&oversized));
        assert!(!bounded_inline_gif(b"GIF89a"));
    }

    #[test]
    fn bounds_streamed_transcode_output_before_appending() {
        let output = b"bounded output";
        assert_eq!(
            read_transcode_output(std::io::Cursor::new(output)),
            Some(output.to_vec())
        );
        assert!(
            read_transcode_output(std::io::Read::take(
                std::io::repeat(0),
                (MAX_INLINE_MEDIA_BYTES + 1) as u64
            ))
            .is_none()
        );
    }

    #[test]
    fn installed_ffmpeg_converter_produces_a_bounded_animation() {
        if !Path::new(SIGNAL_GIF_FFMPEG).is_file() || !Path::new(SIGNAL_GIF_PRLIMIT).is_file() {
            assert!(
                std::env::var_os("SIGNAL_PURPLE_REQUIRE_FFMPEG_TEST").is_none(),
                "CI requires /usr/bin/ffmpeg and /usr/bin/prlimit for converter coverage"
            );
            return;
        }
        let source = std::process::Command::new(SIGNAL_GIF_FFMPEG)
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=16x16:rate=2:duration=1",
                "-frames:v",
                "2",
                "-an",
                "-c:v",
                "mpeg4",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "frag_keyframe+empty_moov",
                "-f",
                "mp4",
                "pipe:1",
            ])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .output()
            .expect("installed FFmpeg did not start");
        assert!(source.status.success());
        assert!(mp4_file_type_box_matches(&source.stdout));

        let gif = transcode_signal_gif_video_blocking(source.stdout)
            .expect("installed FFmpeg did not produce a bounded GIF");
        assert!(bounded_inline_gif(&gif));
        assert!(gif.starts_with(b"GIF89a"));
    }

    #[test]
    fn applies_only_valid_signal_gif_presentations() {
        let attachment = AttachmentPointer {
            file_name: Some("../../shared.MP4".into()),
            content_type: Some("video/mp4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };
        let original = signal_gif_mp4();
        let gif = encoded_gif(1, 1, 2);
        let mut downloaded = DownloadedAttachment::new(3, &attachment, original.clone());

        assert_eq!(
            downloaded.signal_gif_filename.as_deref(),
            Some("shared.gif")
        );
        assert!(downloaded.apply_signal_gif(gif.clone()));
        assert_eq!(downloaded.filename, "shared.gif");
        assert_eq!(downloaded.content_type.as_deref(), Some("image/gif"));
        assert_eq!(downloaded.data, gif);

        let mut invalid = DownloadedAttachment::new(4, &attachment, original.clone());
        assert!(!invalid.apply_signal_gif(b"GIF89a".to_vec()));
        assert_eq!(invalid.data, original);
        assert_eq!(invalid.content_type.as_deref(), Some("video/mp4"));

        let direct = DownloadedAttachment::new(6, &attachment, signal_gif_mp4());
        assert_eq!(direct.signal_gif_filename.as_deref(), Some("shared.gif"));
    }

    #[test]
    fn recognizes_only_declared_image_payloads_for_inline_display() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0];
        let png = b"\x89PNG\r\n\x1a\nrest";
        let gif87a = b"GIF87arest";
        let gif89a = b"GIF89arest";

        assert!(inline_image_matches(Some("image/jpeg"), &jpeg));
        assert!(inline_image_matches(Some("IMAGE/JPEG"), &jpeg));
        assert!(inline_image_matches(Some("image/png"), png));
        assert!(inline_image_matches(Some("IMAGE/PNG"), png));
        assert!(inline_image_matches(Some("image/gif"), gif87a));
        assert!(inline_image_matches(Some("IMAGE/GIF"), gif89a));

        assert!(!inline_image_matches(Some("image/png"), &jpeg));
        assert!(!inline_image_matches(Some("image/jpeg"), png));
        assert!(!inline_image_matches(Some("image/png"), b"\x89PNG"));
        assert!(!inline_image_matches(Some("image/gif"), b"GIF89"));
        assert!(!inline_image_matches(
            Some("image/jpeg; charset=binary"),
            &jpeg
        ));
        assert!(!inline_image_matches(None, &jpeg));

        let mut oversized = vec![0u8; MAX_INLINE_MEDIA_BYTES + 1];
        oversized[..jpeg.len()].copy_from_slice(&jpeg);
        assert!(!inline_image_matches(Some("image/jpeg"), &oversized));
    }

    #[test]
    fn inlines_downloaded_images_in_every_incoming_conversation() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0];

        assert!(should_inline_image(false, Some("image/jpeg"), Some(&jpeg)));
        assert!(!should_inline_image(true, Some("image/jpeg"), Some(&jpeg)));
        assert!(!should_inline_image(
            false,
            Some("application/octet-stream"),
            Some(&jpeg)
        ));
        assert!(!should_inline_image(false, Some("image/jpeg"), None));
    }

    #[test]
    fn downscales_oversized_avatar_and_caches_to_disk() {
        let test_dir = TestDirectory::new("avatar-cache");
        let store_path = test_dir.join("presage.db");
        let cache = AvatarCache::new(store_path.to_str());

        let img = RgbaImage::new(300, 200);
        let mut raw_png = Vec::new();
        img.write_to(&mut Cursor::new(&mut raw_png), ImageFormat::Png)
            .expect("png write should succeed");

        let (processed1, checksum1) = cache.prepare_avatar(raw_png.clone());
        assert!(!processed1.is_empty());
        assert!(!checksum1.is_empty());

        let reader = ImageReader::new(Cursor::new(&processed1))
            .with_guessed_format()
            .expect("format should be guessed");
        let (w, h) = reader.into_dimensions().expect("dimensions should be read");
        assert_eq!(w, 192);
        assert_eq!(h, 128);

        let (processed2, checksum2) = cache.prepare_avatar(raw_png.clone());
        assert_eq!(processed1, processed2);
        assert_eq!(checksum1, checksum2);

        let cache2 = AvatarCache::new(store_path.to_str());
        let (processed3, checksum3) = cache2.prepare_avatar(raw_png);
        assert_eq!(processed1, processed3);
        assert_eq!(checksum1, checksum3);
    }

    #[test]
    fn preserves_small_avatar_without_modification() {
        let cache = AvatarCache::new(None);
        let img = RgbaImage::new(64, 64);
        let mut raw_png = Vec::new();
        img.write_to(&mut Cursor::new(&mut raw_png), ImageFormat::Png)
            .expect("png write should succeed");

        let (processed, checksum) = cache.prepare_avatar(raw_png.clone());
        assert_eq!(processed, raw_png);
        assert_eq!(checksum, hex::encode(Sha256::digest(&raw_png)));
    }

    #[test]
    fn handles_corrupt_avatar_gracefully() {
        let cache = AvatarCache::new(None);
        let corrupt_data = b"definitely not an image".to_vec();
        let (processed, checksum) = cache.prepare_avatar(corrupt_data.clone());
        assert_eq!(processed, corrupt_data);
        assert_eq!(checksum, hex::encode(Sha256::digest(&corrupt_data)));
    }
}
