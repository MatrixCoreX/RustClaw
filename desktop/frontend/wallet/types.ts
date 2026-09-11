export interface WalletAccount {
  id: string;
  name: string;
  public_key: string;
  backed_up: boolean;
}
export interface WalletStatus {
  initialized: boolean;
  unlocked: boolean;
  accounts: WalletAccount[];
}
export type Service = "assets" | "bancor";
export interface Capabilities {
  schema_version: 1;
  protocol: "asset_owner_v1";
  ledger_id: string;
  node_url: string;
  service: Service;
  actions: string[];
}
export interface ReadResult {
  account: string;
  ledger_id: string;
  node_url: string;
  aic_balance_units: string;
  usd_balance_units: string;
  page: number;
  total_pages: number;
  records: HistoryEntry[];
}
export interface HistoryEntry {
  operation_id: string;
  kind: string;
  asset: "AIC" | "USD";
  amount_units: string;
  counterparty: string | null;
  created_at_unix: number;
}
export interface OperationRecord {
  operation_id: string;
  profile_id: string;
  account_id: string;
  public_key: string;
  ledger_id: string;
  node_url: string;
  service: Service;
  kind: string;
  status: string;
  created_at_unix: number;
  receipt_id: string | null;
}
export interface Outcome {
  operation_id: string;
  account: string;
  ledger_id: string;
  status: "succeeded" | "failed" | "pending" | "expired";
  receipt_id: string | null;
}
export type Intent =
  | {
      kind: "bancor_trade";
      side: "buy" | "sell";
      input_units: string;
      slippage_bps: number;
      max_fee_bps: number;
    }
  | {
      kind: "transfer";
      asset: "AIC" | "USD";
      amount_units: string;
      recipient: string;
      memo: string;
      max_fee_bps: number;
    };
export type Terms = Intent & {
  fee_units: string;
  quoted_output_units?: string;
  min_output_units?: string;
};
export interface Confirmation {
  account_name: string;
  public_key: string;
  device_label: string;
  origin: string;
  payload: {
    operation_id: string;
    account: string;
    ledger_id: string;
    node_url: string;
    service: Service;
    expires_at_unix: number;
    terms: Terms;
  };
}
