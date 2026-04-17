-- Channel-aware columns for multi-platform ingress (telegram/whatsapp/ui/wechat).
ALTER TABLE tasks ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat'));
ALTER TABLE tasks ADD COLUMN external_user_id TEXT;
ALTER TABLE tasks ADD COLUMN external_chat_id TEXT;

ALTER TABLE scheduled_jobs ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat'));
ALTER TABLE scheduled_jobs ADD COLUMN external_user_id TEXT;
ALTER TABLE scheduled_jobs ADD COLUMN external_chat_id TEXT;

ALTER TABLE memories ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark', 'wechat'));
ALTER TABLE memories ADD COLUMN external_chat_id TEXT;
