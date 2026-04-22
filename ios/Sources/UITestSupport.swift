import Foundation

struct UITestLaunchConfiguration {
    struct MockTailscaleDevice {
        let name: String
        let host: String
        let address: String?
        let os: String?
        let online: Bool
    }

    let resetStorage: Bool
    let nodeName: String?
    let host: String?
    let port: UInt16
    let autoConnect: Bool
    let tailscalePort: UInt16?
    let tailscaleToken: String?
    let showFloatingBackButton: Bool?
    let forcedTerminalErrorKind: String?
    let mockTailscaleDevices: [MockTailscaleDevice]

    private static func fileConfiguredHostAndPort() -> (host: String, port: UInt16)? {
        let url = URL(fileURLWithPath: "/tmp/boo-ios-ui-tests.env")
        guard let raw = try? String(contentsOf: url, encoding: .utf8) else { return nil }

        var host: String?
        var port: UInt16?
        for line in raw.split(whereSeparator: \.isNewline) {
            let parts = line.split(separator: "=", maxSplits: 1).map(String.init)
            guard parts.count == 2 else { continue }
            switch parts[0] {
            case "BOO_UI_TEST_HOST":
                host = parts[1]
            case "BOO_UI_TEST_PORT":
                port = UInt16(parts[1])
            default:
                break
            }
        }

        guard let host, let port else { return nil }
        return (host, port)
    }

    private static func argumentValue(prefix: String, arguments: [String]) -> String? {
        arguments.first { $0.hasPrefix(prefix) }.map { String($0.dropFirst(prefix.count)) }
    }

    private static func parseMockTailscaleDevices(arguments: [String], env: [String: String]) -> [MockTailscaleDevice] {
        let raw = argumentValue(prefix: "--boo-ui-test-tailscale-devices=", arguments: arguments)
            ?? env["BOO_UI_TEST_TAILSCALE_DEVICES"]
        guard let raw, !raw.isEmpty else { return [] }

        return raw.split(separator: ";").compactMap { entry in
            let parts = entry.split(separator: "|", omittingEmptySubsequences: false).map(String.init)
            guard parts.count >= 5 else { return nil }
            return MockTailscaleDevice(
                name: parts[0],
                host: parts[1],
                address: parts[2].isEmpty ? nil : parts[2],
                os: parts[3].isEmpty ? nil : parts[3],
                online: parts[4] == "1"
            )
        }
    }

    static func current() -> UITestLaunchConfiguration? {
        let env = ProcessInfo.processInfo.environment
        let arguments = ProcessInfo.processInfo.arguments
        let info = Bundle.main.infoDictionary
        let fileConfigured = fileConfiguredHostAndPort()
        let modeEnabled =
            env["BOO_UI_TEST_MODE"] == "1" ||
            arguments.contains("--boo-ui-test-mode") ||
            (info?["BOO_UI_TEST_HOST"] as? String) != nil ||
            (info?["BOO_UI_TEST_PORT"] != nil) ||
            fileConfigured != nil
        guard modeEnabled else { return nil }

        let hostFromArgs = argumentValue(prefix: "--boo-ui-test-host=", arguments: arguments)
        let hostFromEnv = env["BOO_UI_TEST_HOST"]
        let hostFromInfo = info?["BOO_UI_TEST_HOST"] as? String
        let host = hostFromArgs ?? hostFromEnv ?? hostFromInfo ?? fileConfigured?.host

        let portFromArgs = argumentValue(prefix: "--boo-ui-test-port=", arguments: arguments).flatMap(UInt16.init)
        let portFromEnv = env["BOO_UI_TEST_PORT"].flatMap(UInt16.init)
        let portFromInfoString = (info?["BOO_UI_TEST_PORT"] as? String).flatMap(UInt16.init)
        let portFromInfoNumber = (info?["BOO_UI_TEST_PORT"] as? NSNumber)?.uint16Value
        let port = portFromArgs ?? portFromEnv ?? portFromInfoString ?? portFromInfoNumber ?? fileConfigured?.port ?? BooDefaultRemotePort
        let nodeName = argumentValue(prefix: "--boo-ui-test-node-name=", arguments: arguments) ?? env["BOO_UI_TEST_NODE_NAME"]
        let autoConnect = arguments.contains("--boo-ui-test-auto-connect") || env["BOO_UI_TEST_AUTO_CONNECT"] == "1"
        let resetStorage = arguments.contains("--boo-ui-test-reset-storage") || env["BOO_UI_TEST_RESET_STORAGE"] == "1"
        let tailscalePort = argumentValue(prefix: "--boo-ui-test-tailscale-port=", arguments: arguments)
            .flatMap(UInt16.init)
            ?? env["BOO_UI_TEST_TAILSCALE_PORT"].flatMap(UInt16.init)
        let tailscaleToken = argumentValue(prefix: "--boo-ui-test-tailscale-token=", arguments: arguments)
            ?? env["BOO_UI_TEST_TAILSCALE_TOKEN"]
        let showFloatingBackButton = argumentValue(prefix: "--boo-ui-test-show-floating-back-button=", arguments: arguments)
            .flatMap { ["1", "true", "yes"].contains($0.lowercased()) ? true : ["0", "false", "no"].contains($0.lowercased()) ? false : nil }
            ?? env["BOO_UI_TEST_SHOW_FLOATING_BACK_BUTTON"].flatMap { ["1", "true", "yes"].contains($0.lowercased()) ? true : ["0", "false", "no"].contains($0.lowercased()) ? false : nil }
        let forcedTerminalErrorKind = argumentValue(prefix: "--boo-ui-test-terminal-error=", arguments: arguments)
            ?? env["BOO_UI_TEST_TERMINAL_ERROR"]

        return UITestLaunchConfiguration(
            resetStorage: resetStorage,
            nodeName: nodeName,
            host: host,
            port: port,
            autoConnect: autoConnect,
            tailscalePort: tailscalePort,
            tailscaleToken: tailscaleToken,
            showFloatingBackButton: showFloatingBackButton,
            forcedTerminalErrorKind: forcedTerminalErrorKind,
            mockTailscaleDevices: parseMockTailscaleDevices(arguments: arguments, env: env)
        )
    }
}
