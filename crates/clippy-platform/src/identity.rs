#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributionChannel {
    Production,
    Qa,
}

impl DistributionChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Qa => "qa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformIdentity {
    pub application_id: &'static str,
    pub data_namespace: &'static str,
    pub single_instance_key: &'static str,
    pub shortcut_namespace: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentIdentity {
    pub product_name: &'static str,
    pub channel: DistributionChannel,
    pub linux: PlatformIdentity,
    pub windows: PlatformIdentity,
    pub macos: PlatformIdentity,
}

impl DeploymentIdentity {
    pub const fn current_platform(&self) -> &PlatformIdentity {
        #[cfg(target_os = "linux")]
        {
            &self.linux
        }

        #[cfg(target_os = "windows")]
        {
            &self.windows
        }

        #[cfg(target_os = "macos")]
        {
            &self.macos
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        compile_error!("Clippy deployment identity only supports Linux, Windows, and macOS");
    }
}

pub const PRODUCTION_IDENTITY: DeploymentIdentity = DeploymentIdentity {
    product_name: "Clippy",
    channel: DistributionChannel::Production,
    linux: PlatformIdentity {
        application_id: "com.clippy.app",
        data_namespace: "com.clippy.app",
        single_instance_key: "com.clippy.app",
        shortcut_namespace: "com.clippy.app.shortcuts",
    },
    windows: PlatformIdentity {
        application_id: "com.clippy.app",
        data_namespace: "com.clippy.app",
        single_instance_key: "com.clippy.app",
        shortcut_namespace: "com.clippy.app.shortcuts",
    },
    macos: PlatformIdentity {
        application_id: "com.clippy.desktop",
        data_namespace: "com.clippy.desktop",
        single_instance_key: "com.clippy.desktop",
        shortcut_namespace: "com.clippy.desktop.shortcuts",
    },
};

pub const SLINT_QA_IDENTITY: DeploymentIdentity = DeploymentIdentity {
    product_name: "Clippy Slint QA",
    channel: DistributionChannel::Qa,
    linux: PlatformIdentity {
        application_id: "com.clippy.qa.slint",
        data_namespace: "com.clippy.qa.slint",
        single_instance_key: "com.clippy.qa.slint",
        shortcut_namespace: "com.clippy.qa.slint.shortcuts",
    },
    windows: PlatformIdentity {
        application_id: "com.clippy.qa.slint",
        data_namespace: "com.clippy.qa.slint",
        single_instance_key: "com.clippy.qa.slint",
        shortcut_namespace: "com.clippy.qa.slint.shortcuts",
    },
    macos: PlatformIdentity {
        application_id: "com.clippy.qa.slint",
        data_namespace: "com.clippy.qa.slint",
        single_instance_key: "com.clippy.qa.slint",
        shortcut_namespace: "com.clippy.qa.slint.shortcuts",
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_identity_matches_the_existing_platform_identifiers() {
        assert_eq!(PRODUCTION_IDENTITY.linux.application_id, "com.clippy.app");
        assert_eq!(PRODUCTION_IDENTITY.windows.application_id, "com.clippy.app");
        assert_eq!(
            PRODUCTION_IDENTITY.macos.application_id,
            "com.clippy.desktop"
        );
        assert_eq!(PRODUCTION_IDENTITY.channel.as_str(), "production");
    }

    #[test]
    fn current_platform_identity_uses_the_host_namespace() {
        #[cfg(target_os = "linux")]
        assert_eq!(
            PRODUCTION_IDENTITY.current_platform(),
            &PRODUCTION_IDENTITY.linux
        );

        #[cfg(target_os = "windows")]
        assert_eq!(
            PRODUCTION_IDENTITY.current_platform(),
            &PRODUCTION_IDENTITY.windows
        );

        #[cfg(target_os = "macos")]
        assert_eq!(
            PRODUCTION_IDENTITY.current_platform(),
            &PRODUCTION_IDENTITY.macos
        );
    }

    #[test]
    fn slint_qa_is_isolated_from_every_production_namespace() {
        for (production, qa) in [
            (PRODUCTION_IDENTITY.linux, SLINT_QA_IDENTITY.linux),
            (PRODUCTION_IDENTITY.windows, SLINT_QA_IDENTITY.windows),
            (PRODUCTION_IDENTITY.macos, SLINT_QA_IDENTITY.macos),
        ] {
            assert_ne!(qa.application_id, production.application_id);
            assert_ne!(qa.data_namespace, production.data_namespace);
            assert_ne!(qa.single_instance_key, production.single_instance_key);
            assert_ne!(qa.shortcut_namespace, production.shortcut_namespace);
            assert!(qa.application_id.contains(".qa."));
        }
        assert_eq!(SLINT_QA_IDENTITY.channel.as_str(), "qa");
    }
}
