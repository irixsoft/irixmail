export interface DnsStatus {
  state: "unverified" | "verified" | "failing";
  checked_at?: number;
  missing?: string[];
}

export interface Domain {
  id: string;
  name: string;
  aliases: string[];
  enabled: boolean;
  catch_all_account_id: string | null;
  dkim_key_ids: string[];
  dns_status: DnsStatus;
  created_at: number;
}

export interface QueueRecipient {
  address: string;
  status: string;
  lastError?: string | null;
}

export interface QueueMessage {
  id: string;
  sender?: string;
  subject?: string;
  status?: string;
  createdAt?: number;
  nextAttemptAt?: number;
  recipients?: QueueRecipient[];
}

export type IpRuleAction = "allow" | "block";

export interface IpRule {
  id: string;
  cidr: string;
  action: IpRuleAction;
}

export interface DnsRecord {
  kind: string;
  name: string;
  record_type: string;
  value: string;
  ttl: number;
  in_zone: boolean;
}

export interface DnsVerifyResult {
  record: DnsRecord;
  status: string;
}

export interface AppPassword {
  id: string;
  name: string;
  createdAt: number;
  lastUsedAt: number | null;
}

export type Role = "admin" | "user";

export interface Forwarding {
  destinations: string[];
  keep_local_copy: boolean;
}

export interface VacationResponder {
  enabled: boolean;
  subject: string;
  body: string;
  active_from: number | null;
  active_to: number | null;
}

export interface Account {
  id: string;
  local_part: string;
  domain_id: string;
  display_name: string;
  enabled: boolean;
  role: Role;
  aliases: string[];
  forwarding: Forwarding;
  quota_bytes: number;
  quota_messages: number;
  locale: string;
  timezone: string;
  signature: string;
  vacation: VacationResponder;
  created_at: number;
}
