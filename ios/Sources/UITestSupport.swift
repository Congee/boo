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
    let forcedTerminalErrorKind: String?
    let traceActions: Set<String>
    let traceInputCommand: String?
    let traceOutputMarker: String?
    let targetViewedTabIndex: Int?
    let targetViewedTabId: UInt32?
    let forceActiveTerminal: Bool
    let forceOpeningTerminal: Bool
    let terminalOpeningTimeoutSeconds: TimeInterval?
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
        let hostFromInfo = resolvedInfoString(info?["BOO_UI_TEST_HOST"] as? String)
        let rawPortFromInfoString = resolvedInfoString(info?["BOO_UI_TEST_PORT"] as? String)
        let modeEnabled =
            env["BOO_UI_TEST_MODE"] == "1" ||
            arguments.contains("--boo-ui-test-mode") ||
            hostFromInfo != nil ||
            rawPortFromInfoString != nil ||
            (info?["BOO_UI_TEST_PORT"] as? NSNumber) != nil ||
            fileConfigured != nil
        guard modeEnabled else { return nil }

        let hostFromArgs = argumentValue(prefix: "--boo-ui-test-host=", arguments: arguments)
        let hostFromEnv = env["BOO_UI_TEST_HOST"]
        let host = hostFromArgs ?? hostFromEnv ?? hostFromInfo ?? fileConfigured?.host

        let portFromArgs = argumentValue(prefix: "--boo-ui-test-port=", arguments: arguments).flatMap(UInt16.init)
        let portFromEnv = env["BOO_UI_TEST_PORT"].flatMap(UInt16.init)
        let portFromInfoString = rawPortFromInfoString.flatMap(UInt16.init)
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
        let forcedTerminalErrorKind = argumentValue(prefix: "--boo-ui-test-terminal-error=", arguments: arguments)
            ?? env["BOO_UI_TEST_TERMINAL_ERROR"]
        let traceActionsRaw = argumentValue(prefix: "--boo-ui-test-trace-actions=", arguments: arguments)
            ?? env["BOO_UI_TEST_TRACE_ACTIONS"]
        let traceActions = Set(
            (traceActionsRaw ?? "")
                .split(separator: ",")
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
        )
        let traceInputCommand = argumentValue(prefix: "--boo-ui-test-trace-input-command=", arguments: arguments)
            ?? env["BOO_UI_TEST_TRACE_INPUT_COMMAND"]
        let traceOutputMarker = argumentValue(prefix: "--boo-ui-test-trace-output-marker=", arguments: arguments)
            ?? env["BOO_UI_TEST_TRACE_OUTPUT_MARKER"]
        let targetViewedTabIndex = argumentValue(prefix: "--boo-ui-test-target-viewed-tab-index=", arguments: arguments)
            .flatMap(Int.init)
            ?? env["BOO_UI_TEST_TARGET_VIEWED_TAB_INDEX"].flatMap(Int.init)
        let targetViewedTabId = argumentValue(prefix: "--boo-ui-test-target-viewed-tab-id=", arguments: arguments)
            .flatMap(UInt32.init)
            ?? env["BOO_UI_TEST_TARGET_VIEWED_TAB_ID"].flatMap(UInt32.init)
        let forceActiveTerminal = arguments.contains("--boo-ui-test-force-active-terminal")
        let forceOpeningTerminal = arguments.contains("--boo-ui-test-force-opening-terminal")
        let terminalOpeningTimeoutSeconds = argumentValue(prefix: "--boo-ui-test-terminal-opening-timeout=", arguments: arguments)
            .flatMap(TimeInterval.init)
        return UITestLaunchConfiguration(
            resetStorage: resetStorage,
            nodeName: nodeName,
            host: host,
            port: port,
            autoConnect: autoConnect,
            tailscalePort: tailscalePort,
            tailscaleToken: tailscaleToken,
            forcedTerminalErrorKind: forcedTerminalErrorKind,
            traceActions: traceActions,
            traceInputCommand: traceInputCommand,
            traceOutputMarker: traceOutputMarker,
            targetViewedTabIndex: targetViewedTabIndex,
            targetViewedTabId: targetViewedTabId,
            forceActiveTerminal: forceActiveTerminal,
            forceOpeningTerminal: forceOpeningTerminal,
            terminalOpeningTimeoutSeconds: terminalOpeningTimeoutSeconds,
            mockTailscaleDevices: parseMockTailscaleDevices(arguments: arguments, env: env)
        )
    }

    private static func resolvedInfoString(_ value: String?) -> String? {
        guard let value else { return nil }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.contains("$(") else { return nil }
        return trimmed
    }
}
