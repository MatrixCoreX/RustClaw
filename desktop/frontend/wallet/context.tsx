import { createContext, useContext } from "react";
import type { WalletAccount } from "./types";
import type { useAssetAccount } from "./useAssetAccount";

export interface DesktopAssetAccount {
  account: WalletAccount;
  hardwarePublicKey?: string | null;
  runtime: ReturnType<typeof useAssetAccount>;
  statusMessage: string;
  transferFeeLimit?: string;
  setTransferFeeLimit?: (value: string) => void;
}
export const DesktopAssetAccountContext =
  createContext<DesktopAssetAccount | null>(null);
export const useDesktopAssetAccount = () =>
  useContext(DesktopAssetAccountContext);
