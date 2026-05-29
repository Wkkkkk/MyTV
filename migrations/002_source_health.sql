ALTER TABLE sources ADD COLUMN last_checked_at     INTEGER;
ALTER TABLE sources ADD COLUMN last_status          TEXT CHECK(last_status IN ('ok', 'error'));
ALTER TABLE sources ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sources ADD COLUMN failure_reason       TEXT;
