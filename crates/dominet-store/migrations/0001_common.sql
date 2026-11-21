-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Conventions: BIGINT identity keys, timestamptz (UTC), text + CHECK instead of PostgreSQL enums
-- (cheaper to evolve), snake_case names, FKs always named by the referenced table.

CREATE FUNCTION set_updated_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$;
