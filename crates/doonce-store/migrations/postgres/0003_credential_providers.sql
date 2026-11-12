ALTER TABLE credentials ADD COLUMN provider TEXT NOT NULL DEFAULT 'local';
ALTER TABLE credentials ADD COLUMN external_ref TEXT;
