//! Deployment topology and data-residency policy shared by every binary.

use std::fmt;
use std::str::FromStr;

/// How this installation is deployed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeploymentMode {
    /// Multi-tenant SaaS control plane with optional remote data planes.
    Saas,
    /// Customer/VPC deployment, possibly connected to a SaaS control plane.
    #[default]
    OnPrem,
    /// Fully disconnected deployment with no external network dependencies.
    AirGapped,
}

impl DeploymentMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "saas" | "cloud" | "multi-tenant" => Some(Self::Saas),
            "onprem" | "on-prem" | "on-premise" | "on-premises" | "vpc" => Some(Self::OnPrem),
            "airgap" | "air-gap" | "airgapped" | "air-gapped" | "offline" => Some(Self::AirGapped),
            _ => None,
        }
    }

    pub fn from_env() -> Self {
        if let Ok(value) = std::env::var("DOONCE_DEPLOYMENT_MODE")
            && !value.trim().is_empty()
            && let Some(mode) = Self::parse(&value)
        {
            return mode;
        }
        let air_gapped = std::env::var("DOONCE_AIRGAPPED").ok().is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
        if air_gapped {
            Self::AirGapped
        } else {
            Self::OnPrem
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Saas => "saas",
            Self::OnPrem => "onprem",
            Self::AirGapped => "airgapped",
        }
    }

    pub fn is_air_gapped(self) -> bool {
        self == Self::AirGapped
    }

    pub fn allows_external_network(self) -> bool {
        !self.is_air_gapped()
    }
}

impl FromStr for DeploymentMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or_else(|| {
            format!("unknown deployment mode \"{value}\" (use saas, onprem or airgapped)")
        })
    }
}

impl fmt::Display for DeploymentMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Default region for single-region installations and legacy tenants.
pub const DEFAULT_REGION: &str = "global";

pub fn region_from_env() -> String {
    std::env::var("DOONCE_REGION")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REGION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes() {
        assert_eq!(DeploymentMode::parse("SaaS"), Some(DeploymentMode::Saas));
        assert_eq!(
            DeploymentMode::parse("on-prem"),
            Some(DeploymentMode::OnPrem)
        );
        assert_eq!(
            DeploymentMode::parse("air_gapped"),
            Some(DeploymentMode::AirGapped)
        );
        assert_eq!(DeploymentMode::parse("unknown"), None);
    }

    #[test]
    fn air_gapped_has_no_external_network() {
        assert!(!DeploymentMode::AirGapped.allows_external_network());
        assert!(DeploymentMode::OnPrem.allows_external_network());
    }
}
