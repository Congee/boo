#[cfg(test)]
mod tests {
    use crate::remote::RemoteConfig;

    #[test]
    fn remote_config_defaults_to_loopback_without_advertising() {
        let config = RemoteConfig {
            port: crate::config::DEFAULT_REMOTE_PORT,
            bind_address: None,
            service_name: "boo".to_string(),
        };
        assert_eq!(config.effective_bind_address(), "127.0.0.1");
        assert!(!config.should_advertise());
    }

    #[test]
    fn remote_config_explicit_bind_address_advertises() {
        let config = RemoteConfig {
            port: crate::config::DEFAULT_REMOTE_PORT,
            bind_address: Some("192.0.2.5".to_string()),
            service_name: "boo".to_string(),
        };
        assert_eq!(config.effective_bind_address(), "192.0.2.5");
        assert!(config.should_advertise());
    }
}
