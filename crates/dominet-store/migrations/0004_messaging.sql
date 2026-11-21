-- SPDX-License-Identifier: AGPL-3.0-or-later
CREATE TABLE message (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    sender_id        BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    subject          TEXT        NOT NULL CHECK (length(subject) BETWEEN 1 AND 200),
    body             TEXT        NOT NULL,
    channel          TEXT        NOT NULL DEFAULT 'email' CHECK (channel IN ('email', 'sms', 'both')),
    status           TEXT        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'sending', 'sent')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at          TIMESTAMPTZ,
    recipients_count INTEGER     NOT NULL DEFAULT 0 CHECK (recipients_count >= 0)
);
CREATE INDEX message_sender_idx ON message (sender_id, created_at DESC);
CREATE INDEX message_created_idx ON message (created_at DESC);

CREATE TABLE message_recipient (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_id   BIGINT      NOT NULL REFERENCES message (id) ON DELETE CASCADE,
    recipient_id BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email_status TEXT        NOT NULL DEFAULT 'na' CHECK (email_status IN ('pending', 'sent', 'failed', 'na')),
    sms_status   TEXT        NOT NULL DEFAULT 'na' CHECK (sms_status IN ('pending', 'sent', 'failed', 'na')),
    error        TEXT        NOT NULL DEFAULT '',
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (message_id, recipient_id)
);
CREATE INDEX message_recipient_recipient_idx ON message_recipient (recipient_id);
CREATE TRIGGER message_recipient_set_updated_at BEFORE UPDATE ON message_recipient
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
