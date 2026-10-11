-- A workspace's history names each kind of entry. Asking a workspace to
-- run, freeze or stop was "requested", as asking for the workspace was;
-- it's its own kind, and its detail is the state.
UPDATE activity SET kind = 'state-set', detail = substr(detail, 8)
WHERE kind = 'requested' AND detail LIKE 'set to %';
