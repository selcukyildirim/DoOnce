//! Role-based access control.
//!
//! Authorization is deny-by-default. Every API handler must call
//! [`role_allows`] (or a higher-level policy check) before touching a tenant
//! resource. Roles are intentionally coarse; finer scopes can be layered on
//! top via API key scopes.

use doonce_core::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    TenantRead,
    TenantWrite,
    TenantDelete,
    UserRead,
    UserManage,
    ApiKeyRead,
    ApiKeyWrite,
    WorkflowRead,
    WorkflowWrite,
    WorkflowRun,
    RunRead,
    RunCancel,
    TaskRead,
    TaskWrite,
    CredentialRead,
    CredentialWrite,
    ScheduleRead,
    ScheduleWrite,
    WebhookRead,
    WebhookWrite,
    LlmConfigRead,
    LlmConfigWrite,
    ArtifactRead,
    ArtifactWrite,
    AuditRead,
    SettingsRead,
    SettingsWrite,
    BrowserSessionRead,
    BrowserSessionWrite,
    ConnectorRead,
    ConnectorWrite,
}

pub fn role_allows(role: Role, permission: Permission) -> bool {
    match role {
        Role::Owner => true,
        Role::Admin => !matches!(permission, Permission::TenantDelete),
        Role::Operator => matches!(
            permission,
            Permission::TenantRead
                | Permission::WorkflowRead
                | Permission::WorkflowRun
                | Permission::RunRead
                | Permission::RunCancel
                | Permission::TaskRead
                | Permission::TaskWrite
                | Permission::ScheduleRead
                | Permission::ScheduleWrite
                | Permission::ArtifactRead
                | Permission::ArtifactWrite
                | Permission::BrowserSessionRead
                | Permission::BrowserSessionWrite
                | Permission::CredentialRead
                | Permission::ConnectorRead
        ),
        Role::Viewer => matches!(
            permission,
            Permission::TenantRead
                | Permission::WorkflowRead
                | Permission::RunRead
                | Permission::TaskRead
                | Permission::ScheduleRead
                | Permission::ArtifactRead
                | Permission::BrowserSessionRead
                | Permission::ConnectorRead
        ),
        Role::Auditor => matches!(
            permission,
            Permission::TenantRead
                | Permission::WorkflowRead
                | Permission::RunRead
                | Permission::TaskRead
                | Permission::ArtifactRead
                | Permission::AuditRead
                | Permission::SettingsRead
                | Permission::ConnectorRead
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_can_do_everything() {
        assert!(role_allows(Role::Owner, Permission::TenantDelete));
        assert!(role_allows(Role::Owner, Permission::CredentialWrite));
    }

    #[test]
    fn least_privilege() {
        assert!(!role_allows(Role::Viewer, Permission::WorkflowWrite));
        assert!(!role_allows(Role::Operator, Permission::CredentialWrite));
        assert!(!role_allows(Role::Auditor, Permission::WorkflowRun));
        assert!(role_allows(Role::Auditor, Permission::AuditRead));
        assert!(role_allows(Role::Operator, Permission::WorkflowRun));
    }
}
