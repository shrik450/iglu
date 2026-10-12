-- Writes are checked by where the browser says they came from, not a token.
ALTER TABLE web_session DROP COLUMN csrf;
