-- SPDX-License-Identifier: AGPL-3.0-or-later
-- R5 lot 1 (ADR 0037) : jeton Android hache, file d'envoi des SMS.

-- Le jeton n'est plus stocke en clair : on garde l'empreinte SHA-256 (hexadecimal) du texte canonique de l'UUID
-- (minuscules, tirets). Les jetons existants sont hachees sur place : l'application Android reste valide.
ALTER TABLE android_device ADD COLUMN token_hash TEXT;
UPDATE android_device SET token_hash = encode(sha256(convert_to(token::text, 'UTF8')), 'hex');
ALTER TABLE android_device ALTER COLUMN token_hash SET NOT NULL;
ALTER TABLE android_device ADD CONSTRAINT android_device_token_hash_key UNIQUE (token_hash);
ALTER TABLE android_device ADD CONSTRAINT android_device_token_hash_check CHECK (token_hash ~ '^[0-9a-f]{64}$');
ALTER TABLE android_device DROP COLUMN token;

-- Chaque SMS demande a un appareil (depuis le navigateur ou la messagerie) a son propre identifiant : c'est lui que l'application
-- renvoie dans `sms_sent_ack.message_id`. Python reutilisait la cle d'un SmsMessage ou d'un MessageRecipient, ce qui faisait
-- collisionner les accuses des deux origines.
CREATE TABLE sms_outbox (
    id                   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id            BIGINT      NOT NULL REFERENCES android_device (id) ON DELETE CASCADE,
    to_number            TEXT        NOT NULL CHECK (length(to_number) BETWEEN 1 AND 30),
    body                 TEXT        NOT NULL,
    -- Origine : une ligne de `sms_message` (envoi depuis le navigateur) ou un destinataire de la messagerie.
    sms_message_id       BIGINT REFERENCES sms_message (id) ON DELETE CASCADE,
    message_recipient_id BIGINT REFERENCES message_recipient (id) ON DELETE CASCADE,
    status               TEXT        NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'failed')),
    error                TEXT        NOT NULL DEFAULT '',
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    acked_at             TIMESTAMPTZ,
    CONSTRAINT sms_outbox_one_origin CHECK ((sms_message_id IS NOT NULL) <> (message_recipient_id IS NOT NULL))
);
CREATE INDEX sms_outbox_pending_idx ON sms_outbox (created_at) WHERE status = 'pending';
CREATE INDEX sms_outbox_device_idx ON sms_outbox (device_id);
