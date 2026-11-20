# Handover: Pre-Key Misalignment, SQLite Contention, and Envelope Drops

Date: 2026-11-14
Status: Phase 1A, 1B, 1C and 2B done (2026-11-15). Remaining: live verification.
Components: `adrighem/presage`, `libsignal-service-rs`, `presage-store-sqlite`, `signal-purple`

---

## 1. Context and Problem Statement

Users of `signal-purple` experienced recurring session desynchronization ("pre-key misalignment"), manifested as:
- "Bad encrypted message" / `DecryptionError` on incoming messages.
- Stalled ratchets where the sender advances their Double Ratchet chain while the receiver cannot decrypt.
- Starvation and timeouts during system bootup due to SQLite connection pool limits.

### Root Causes Identified
1. **Single Connection Pool Constraint**: `presage-store-sqlite` pins `max_connections = 1` to prevent SQLite write-lock concurrency errors. Any concurrency spike (avatar fetches, outbox retries, contact sync) starves critical protocol queries.
2. **Silent Envelope Dropping in Presage** (`presage/src/manager/registered.rs:1825`):
   When `open_envelope` encounters a transient database timeout (`PoolTimedOut`, SQLite `BUSY`), Presage logs an error and skips the envelope. The WebSocket stream moves to the next envelope, causing the sender's ratchet to advance while the receiver's state is lost.
3. **Non-Atomic Pre-Key Deletion**:
   In `libsignal-protocol-rs` (`message_decrypt_prekey`), `remove_pre_key` precedes `store_session`. In `presage-store-sqlite`, each is an auto-commit query. A failure or timeout during `store_session` deletes the pre-key without saving the ratchet session.
4. **Unbatched Pre-Key Replenishment**:
   `libsignal-service-rs` (`replenish_pre_keys`) performs 202 unbatched queries (100 EC + 100 Kyber + 1 signed pre-key) on startup, monopolizing the database connection.

---

## 2. Gemini 4 Architectural Review Findings

The proposed mitigation plan was submitted for frontier architectural review (Gemini 4). Key conclusions:

1. **WebSocket Redelivery on Disconnect (Confirmed Safe)**:
   - Traced in `libsignal-service-rs/src/messagepipe.rs` (lines 58-64 and 105).
   - Presage defers WebSocket ACKs via `pending_ack = Some((responder, response));`. The ACK frame is dispatched only when polling for the *subsequent* envelope.
   - Terminating the stream or dropping the WebSocket connection on transient envelope failure guarantees that no ACK is transmitted. The Signal server retains the message and redelivers it upon reconnect.
2. **Poison Message Guardrail (Mandatory)**:
   - Disconnecting on permanent cryptographic errors (corrupt payload, signature failure) would trigger an infinite disconnect/reconnect loop.
   - Classification requirement: Only transient storage errors (`PoolTimedOut`, SQLite `BUSY`/`LOCKED`, IO timeouts) trigger retry and disconnect. Cryptographic failures must be acknowledged and skipped or reported.
3. **Deferred Deletion Pattern for Pre-Keys**:
   - Because `libsignal-protocol-rs` traits decouple `remove_pre_key` from `store_session`, wrapping them in an external transaction is blocked by trait boundaries.
   - Recommended solution: Stash `remove_pre_key` IDs in memory within `SqliteProtocolStore`. When `store_session` executes, persist the session and delete all queued pre-keys inside a single `BEGIN IMMEDIATE` transaction.
4. **Batch Pre-Key Replenishment API**:
   - Extend `PreKeysStore` in the `libsignal-service-rs` fork with `save_pre_keys` and `save_kyber_pre_keys` batch methods, executed in a single transaction in `presage-store-sqlite`.

---

## 3. Implementation Plan

### Phase 1: Upstream Fork Fixes (`adrighem/presage`)

#### 1A. Resilient Envelope Handling in `presage/src/manager/registered.rs`
- In the envelope processing loop:
  - If `open_envelope` returns a transient store error (`PoolTimedOut`, SQLite busy):
    - Retry up to 3 times with exponential backoff (e.g. 50ms, 150ms, 300ms).
    - If retries fail, terminate the incoming message stream / close the WebSocket without acknowledging the envelope.
  - If `open_envelope` returns a cryptographic or permanent error:
    - Log warning, emit decryption failure event if applicable, and proceed to next envelope so the ACK frame is sent.

#### 1B. Deferred Pre-Key Deletion in `presage-store-sqlite/src/protocol.rs`
- Add `pending_pre_keys: Arc<Mutex<Vec<u32>>>` and `pending_kyber_pre_keys: Arc<Mutex<Vec<u32>>>` to `SqliteProtocolStore`.
- `remove_pre_key`: Append ID to `pending_pre_keys`; filter out pending keys in `get_pre_key`.
- `store_session`: Open `self.begin_write().await?` (`BEGIN IMMEDIATE`), write the session record, delete all IDs in `pending_pre_keys`, and commit.

#### 1C. Batch Pre-Key Storage in `libsignal-service-rs` and `presage-store-sqlite`
- Add batch save methods to `PreKeysStore` trait.
- Implement multi-row SQL insert in `presage-store-sqlite` wrapped in a single transaction.
- Update `replenish_pre_keys` in `libsignal-service-rs` to use batch operations.

---

### Phase 2: Client Traffic Shaping (`signal-purple`)

#### 2A. Database Coordination & Pacing
- Ensure background operations (avatar downloads, link preview fetching, outbox delivery retries) defer or yield when `receive_messages` is actively processing a queue.
- Increase SQLite busy timeout in `SqliteConnectOptions` to 5000 ms on connection opening.

#### 2B. Error Classification Verification
- Retain transient error classifications in `rust/signal-core/src/store/errors.rs` and `rust/signal-core/src/backend/coordinator.rs` to avoid user-facing error dialogs during temporary lock contention.

---

## 4. Verification and Validation Criteria

1. **Unit Tests**:
   - Exercise `SqliteProtocolStore` deferred deletion: verify that failing `store_session` preserves the pre-key in SQLite.
   - Exercise batch pre-key insertion against SQLite store.
2. **Contention Stress Test**:
   - Simulate heavy concurrent read/write load against `SqliteStore` during incoming envelope processing to verify retry and reconnect behaviors without envelope loss.
3. **Protocol Interoperability**:
   - Verify clean re-connection and message redelivery from Signal test servers without message loss or ratchet corruption.

---

## 5. Implementation Status (2026-11-15)

| Item | State | Where |
|------|-------|-------|
| 1A Resilient envelope handling | Merged | adrighem/presage#1 (`7974c9aa3`) |
| 1B Deferred pre-key deletion | Merged | adrighem/presage#1 (`7974c9aa3`) |
| 1C Batch pre-key replenishment | Done | adrighem/libsignal-service-rs#1, adrighem/presage#2, adrighem/signal-purple#124. Measured ~1.6 s auto-commit vs ~15 ms batched (NVMe/btrfs) |
| 2A Traffic shaping | Partial | 1.5 s avatar fetch delay (shutdown-aware); busy timeout item dropped |
| 2B Error classification | Merged | adrighem/signal-purple#122, #123 (store `is_transient()` reused) |

### Deviations from the plan

- **Kyber pre-key use deferred too.** libsignal v0.99 `message_decrypt_prekey` runs `save_identity`, `mark_kyber_pre_key_used`, `remove_pre_key`, `store_session` in that order. Both pre-key calls are queued. Last-resort base-key reuse is still rejected at mark time, checked against committed and queued uses.
- **No filtering of pending keys in `get_pre_key`.** A failed `store_session` drops the queue so the redelivered message finds its pre-key; filtering would make that retry fail.
- **Pending queue is `Arc`-shared across clones.** libsignal-service passes a separate clone of the store for each trait argument. ACI and PNI stores, and each `ServiceCipher`, get their own queue.
- **Typed transient classification, no string matching.** New `presage::store::TransientStoreError`; presage-store-sqlite wraps transient sqlx errors (pool timeout, IO, SQLite primary code BUSY/LOCKED) in `SignalProtocolError::ApplicationCallbackError`. The receive loop checks `TransientStoreError::is_cause_of`. Avoids a poison message being misread as transient.
- **Stream end instead of explicit WebSocket close.** Each `receive_messages` opens a fresh websocket; ending the stream makes signal-purple reconnect (recovery backoff), which drops the old socket and triggers redelivery.
- **Busy timeout unchanged.** sqlx already defaults `busy_timeout` to 5 s; with `max_connections(1)` the pool acquire timeout is the effective limit.

### Tests added (adrighem/presage#1)

- End-to-end: a real libsignal pre-key message decrypted into `SqliteProtocolStore` survives a failed `store_session` (injected trigger) and decrypts on redelivery. Fails against the old immediate deletion.
- Contention: a second connection holds the write lock; the decrypt error is `TransientStoreError` and both pre-keys survive.
- Retry policy: recovers after transient errors, gives up (`None`, no ACK) after 3 delays, returns permanent errors on the first attempt.
- Queue semantics: removal waits for the session, a failed session write drops the queue, Kyber base-key reuse rejected while queued and once committed.

### Remaining

1. Live verification (handover item 3): no "Bad encrypted message" after bootup contention.
2. Fork CI enabled 2026-11-15 on adrighem/presage and adrighem/libsignal-service-rs; pre-existing lint debt fixed (libsignal-service-rs#2, presage#2).
3. 2A coordination (replace the 1.5 s avatar delay) only if contention persists after 1C.
