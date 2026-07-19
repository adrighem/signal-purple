# Third-party licenses

The original Purple adapter is GPL-3.0-or-later. The Rust backend links the
following AGPL-3.0-only projects at exact pinned revisions:

| Dependency | Revision | License |
| --- | --- | --- |
| [Presage fork](https://github.com/adrighem/presage) | `c8ee98cf944897812d9375effb98657f537e8b09` (parent: upstream `63482efd0cbdc0780baf0650517c7d55f1cac05d`) | AGPL-3.0-only |
| [libsignal-service-rs](https://github.com/whisperfish/libsignal-service-rs) | `fd481655d481388ebfe3a1fdb85a668b4876d592` | AGPL-3.0-only |
| [Signal libsignal](https://github.com/signalapp/libsignal) | tag `v0.94.4`, commit `46d867c986f66201e34e7ae20ce423eec742bf3f` | AGPL-3.0-only |
| Signal curve25519-dalek fork | tag `signal-curve25519-4.1.3` | BSD-3-Clause |
| Whisperfish rusqlite fork | `2a42b3354c9194700d08aa070f70a131a470e7dc` | MIT |

The complete transitive dependency list and checksums are recorded in
[`rust/signal-core/Cargo.lock`](rust/signal-core/Cargo.lock). Cargo package
license metadata remains authoritative for transitive crates.

GNU GPLv3 section 13 permits GPLv3 and AGPLv3 code to be combined, while the
AGPL section 13 requirements apply to the combination. The original C code does
not become relicensed, but distributing the combined plugin/backend requires
compliance with both applicable licenses. This is project documentation, not
legal advice.

Full license texts are in [`LICENSE`](LICENSE) and
[`LICENSES/AGPL-3.0-only.txt`](LICENSES/AGPL-3.0-only.txt).
