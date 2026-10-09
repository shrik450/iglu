-- Stored JSON from before columns and agents. Running instances gained the
-- state of their columns: forget the old observations, which hosts report
-- again on the next pass. Built images gained the agents they declare:
-- images from before declared none.
UPDATE workspace SET observed = NULL, observed_at = NULL WHERE observed IS NOT NULL;

UPDATE env_revision SET built = json_set(built, '$.agents', json('[]'))
    WHERE built IS NOT NULL AND json_type(built, '$.agents') IS NULL;
