import Foundation

struct UITestLaunchConfiguration {
    let resetStorage: Bool
    let nodeName: String?
    let host: String?
    let port: UInt16
    let authKey: String
    let autoConnect: Bool

    private static func argumentValue(prefix: String, arguments: [String]) -> String? {
        arguments.first { $0.hasPrefix(prefix) }.map { String($0.dropFirst(prefix.count)) }
    }

    static func current() -> UITestLaunchConfiguration? {
        let env = ProcessInfo.processInfo.environment
        let arguments = ProcessInfo.processInfo.arguments
        let modeEnabled = env["BOO_UI_TEST_MODE"] == "1" || arguments.contains("--boo-ui-test-mode")
        guard modeEnabled else { return nil }

        let host = argumentValue(prefix: "--boo-ui-test-host=", arguments: arguments) ?? env["BOO_UI_TEST_HOST"]
        let port = argumentValue(prefix: "--boo-ui-test-port=", arguments: arguments)
            .flatMap(UInt16.init)
            ?? env["BOO_UI_TEST_PORT"]
                .flatMap(UInt16.init)
            ?? 7337
        let nodeName = argumentValue(prefix: "--boo-ui-test-node-name=", arguments: arguments) ?? env["BOO_UI_TEST_NODE_NAME"]
        let authKey = argumentValue(prefix: "--boo-ui-test-auth-key=", arguments: arguments) ?? env["BOO_UI_TEST_AUTH_KEY"] ?? ""
        let autoConnect = arguments.contains("--boo-ui-test-auto-connect") || env["BOO_UI_TEST_AUTO_CONNECT"] == "1"
        let resetStorage = arguments.contains("--boo-ui-test-reset-storage") || env["BOO_UI_TEST_RESET_STORAGE"] == "1"

        return UITestLaunchConfiguration(
            resetStorage: resetStorage,
            nodeName: nodeName,
            host: host,
            port: port,
            authKey: authKey,
            autoConnect: autoConnect
        )
    }
}
