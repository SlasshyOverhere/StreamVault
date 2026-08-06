import { invoke } from "@tauri-apps/api/core";

export interface CfAccount {
  id: string;
  name: string;
}

export interface DeployResult {
  url: string;
  account_id: string;
  script_name: string;
  subdomain: string;
}

export function listAccounts(apiToken: string): Promise<CfAccount[]> {
  return invoke<CfAccount[]>("cf_list_accounts", { apiToken });
}

export function deployRelay(apiToken: string, accountId: string): Promise<DeployResult> {
  return invoke<DeployResult>("cf_deploy_relay", { apiToken, accountId });
}

export function deleteRelay(apiToken: string, accountId: string): Promise<void> {
  return invoke("cf_delete_relay", { apiToken, accountId });
}

export function relayStatus(apiToken: string, accountId: string): Promise<boolean> {
  return invoke<boolean>("cf_relay_status", { apiToken, accountId });
}
