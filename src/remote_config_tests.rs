#[cfg(test)]
mod tests {
    use crate::remote::RemoteConfig;

    #[test]
    fn remote_config_defaults_authless_tcp_to_loopback_without_advertising() {
        let config = RemoteConfig {
            port: 7337,
            bind_address: None,
            auth_key: None,
            allow_insecure_no_auth: false,
            service_name: "boo".to_string(),
            cert_chain_path: None,
            cert_key_path: None,
        };
        assert_eq!(config.effective_bind_address(), "127.0.0.1");
        assert!(!config.should_advertise());
    }

    #[test]
    fn remote_config_defaults_authenticated_tcp_to_public_bind_with_advertising() {
        let config = RemoteConfig {
            port: 7337,
            bind_address: None,
            auth_key: Some("secret".to_string()),
            allow_insecure_no_auth: false,
            service_name: "boo".to_string(),
            cert_chain_path: None,
            cert_key_path: None,
        };
        assert_eq!(config.effective_bind_address(), "0.0.0.0");
        assert!(config.should_advertise());
    }

    #[test]
    fn remote_config_explicit_bind_address_overrides_defaults() {
        let config = RemoteConfig {
            port: 7337,
            bind_address: Some("192.0.2.5".to_string()),
            auth_key: None,
            allow_insecure_no_auth: false,
            service_name: "boo".to_string(),
            cert_chain_path: None,
            cert_key_path: None,
        };
        assert_eq!(config.effective_bind_address(), "192.0.2.5");
        assert!(config.should_advertise());
        assert!(config.rejects_public_authless_bind());
    }

    #[test]
    fn remote_config_allows_explicit_insecure_public_bind_when_acknowledged() {
        let config = RemoteConfig {
            port: 7337,
            bind_address: Some("192.0.2.5".to_string()),
            auth_key: None,
            allow_insecure_no_auth: true,
            service_name: "boo".to_string(),
            cert_chain_path: None,
            cert_key_path: None,
        };
        assert_eq!(config.effective_bind_address(), "192.0.2.5");
        assert!(config.should_advertise());
        assert!(!config.rejects_public_authless_bind());
    }
}
