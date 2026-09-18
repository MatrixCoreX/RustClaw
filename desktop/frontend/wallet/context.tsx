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
  fullHistory?: boolean;
}
export const DesktopAssetAccountContext =
  createContext<DesktopAssetAccount | null>(null);
export const useDesktopAssetAccount = () =>
  useContext(DesktopAssetAccountContext);

// Presentation follows the entry point, independently of the selected account.
export const StandaloneAssetViewContext = createContext(false);
export const useStandaloneAssetView = () => useContext(StandaloneAssetViewContext);
