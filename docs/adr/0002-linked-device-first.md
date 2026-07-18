# ADR 0002: Linked-device mode first

Status: accepted, 2026-07-18.

The first product path links to an existing Signal account by QR. Primary
registration, SMS/voice verification, and active phone-number discovery are
deferred.

This follows Signal's desktop device model and limits initial account-lifecycle
risk. A fresh store links automatically; a registered store must never be
silently cleared or relinked.
