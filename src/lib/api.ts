import { invoke } from "@tauri-apps/api/core";

export interface Account {
  id: string;
  provider: string;
  display_name: string;
  email: string;
  role: string;
  aws_account_id: string;
  active: boolean;
  has_credentials: boolean;
  access_expires_at: number;
  refresh_expires_at: number;
  refresh_failing_since: unknown;
  last_used_at: string | null;
  error: string | null;
}

export const listAccounts = () => invoke<Account[]>("list_accounts");
