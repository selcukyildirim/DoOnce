export interface Tenant {
  id: string;
  name: string;
  slug: string;
  status: string;
  region: string;
}

export interface Principal {
  tenant: Tenant;
  role: string;
  userId?: string | null;
  scopes: string[];
  apiKeyId?: string | null;
  authMethod: "api_key" | "session" | "dev";
  mfaRequired: boolean;
}

export interface ProviderInfo {
  id: string;
  name: string;
  kind: string;
  issuer?: string;
  clientId?: string;
  scopes?: string[];
  enabled?: boolean;
  createdAt?: string;
}

export interface MfaStatus {
  totpEnabled: boolean;
  pending: boolean;
  sessionMfaVerified: boolean;
}

export interface WorkflowRecord {
  id: string;
  slug: string;
  name: string;
  currentVersion: number;
  createdAt: string;
  updatedAt: string;
}

export interface WorkflowDetail {
  workflow: WorkflowRecord;
  definition: unknown;
}

export interface Task {
  id: string;
  prompt: string;
  url?: string | null;
  status: string;
  output?: unknown;
  failureReason?: string | null;
  maxSteps: number;
  createdAt: string;
}

export interface TaskAction {
  id: string;
  tenantId: string;
  taskId: string;
  seq: number;
  action: unknown;
  reasoning?: string | null;
  screenshotArtifactId?: string | null;
  status: string;
  createdAt: string;
}

export interface LlmConfig {
  id: string;
  provider: string;
  model: string;
  baseUrl?: string | null;
  hasApiKey: boolean;
  capabilities?: Record<string, unknown>;
  isDefault: boolean;
  createdAt: string;
  updatedAt: string;
}
export interface Run {
  id: string;
  workflowId: string;
  status: string;
  trigger: string;
  createdAt: string;
  finishedAt?: string | null;
  error?: string | null;
  outputs?: unknown;
}

export interface BlockRun {
  id: string;
  blockId: string;
  status: string;
  error?: string | null;
  finishedAt?: string | null;
}

export interface Credential {
  id: string;
  name: string;
  kind: string;
  provider: string;
  externalRef?: string | null;
  username?: string | null;
  createdAt: string;
}

export interface Connector {
  id: string;
  name: string;
  region: string;
  status: string;
  version?: string | null;
  lastSeenAt?: string | null;
  createdAt: string;
}

export interface Schedule {
  id: string;
  workflowId: string;
  cron: string;
  timezone: string;
  enabled: boolean;
  nextRunAt?: string | null;
  lastStatus?: string | null;
}

export interface AuditLog {
  id: string;
  action: string;
  target: string;
  actorId?: string | null;
  createdAt: string;
}

export interface ApiKey {
  id: string;
  name: string;
  prefix: string;
  scopes: string[];
  lastUsedAt?: string | null;
  revokedAt?: string | null;
  createdAt: string;
}

export interface Approval {
  id: string;
  runId?: string | null;
  taskId?: string | null;
  stepId: string;
  summary: string;
  reason: string;
  status: string;
  createdAt: string;
}

export interface Webhook {
  id: string;
  url: string;
  events: string[];
  enabled: boolean;
  createdAt: string;
}

export interface Metrics {
  workflows: number;
  tasks: number;
  runs: number;
  runsCompleted: number;
  runsFailed: number;
  runsQueued: number;
  tasksQueued: number;
  pendingApprovals: number;
  artifacts: number;
  successRate: number;
  avgRunDurationMs?: number | null;
}

export interface Artifact {
  id: string;
  kind: string;
  filename: string;
  contentType: string;
  size: number;
  createdAt: string;
}
