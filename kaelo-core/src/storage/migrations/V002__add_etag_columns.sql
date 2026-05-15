-- Add ETag and Last-Modified columns for conditional re-fetches.
ALTER TABLE url_cache ADD COLUMN etag TEXT;
ALTER TABLE url_cache ADD COLUMN last_modified TEXT;
