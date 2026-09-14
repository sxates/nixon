-- specs/0052: bound the pre-call-prep retry loop.
--
-- prep_jobs.rs recorded status='failed' with no fingerprint and no counter, deliberately so
-- the next pass retries. There was no give-up, so a permanently-failing brief re-ran every
-- 30 minutes forever, each attempt burning up to 3 x 300s of GPU. This counter lets the
-- background pass stop after 3 consecutive failures until the inputs change.
--
-- Reset to 0 on success, or whenever source_fingerprint changes (new input gets fresh
-- attempts). A manual retry from the activity indicator also clears it.
ALTER TABLE meeting_briefs ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
