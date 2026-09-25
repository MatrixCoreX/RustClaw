-- Migration id: 028_conversation_input_decision_kind_v1
--
-- Records the planner's machine decision category independently from its
-- receipt reference. Checkpoint restore can therefore omit lifecycle controls
-- that already took effect without parsing user text or an opaque identifier.
ALTER TABLE conversation_inputs ADD COLUMN decision_kind TEXT;
