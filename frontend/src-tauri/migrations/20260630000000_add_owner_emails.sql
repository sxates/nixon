-- specs/0018: the source of truth for "which invite addresses are ME (the device owner)".
-- EventKit's per-attendee isCurrentUser() only flags the calendar account's own address, so
-- aliases / personal / delegated addresses are missed and get seeded as ordinary
-- participants. These rows say "this address resolves to the owner person (person-owner-self)".
-- Multi-valued by design (work + personal + aliases). Email is stored NORMALIZED: lowercased
-- and trimmed at the write boundary, so the PK does the dedupe and lookups are exact.
-- The owner person keeps email = NULL; this table is the multi-email home (so it never
-- collides under people's partial-unique idx_people_email).
-- Additive, idempotent, forward-only; no PRAGMA foreign_keys (the merge cascade is explicit
-- in Rust — see people::merge::merge_person_into_owner).
CREATE TABLE IF NOT EXISTS owner_emails (
    email      TEXT PRIMARY KEY,   -- normalized: lower(trim(email))
    created_at TEXT NOT NULL
);
