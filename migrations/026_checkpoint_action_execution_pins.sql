ALTER TABLE task_checkpoint_actions ADD COLUMN execution_binding_json TEXT;
ALTER TABLE task_checkpoint_actions ADD COLUMN approval_binding_json TEXT;
ALTER TABLE task_checkpoint_actions ADD COLUMN instruction_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE task_checkpoint_actions ADD COLUMN execution_epoch INTEGER NOT NULL DEFAULT 0;
