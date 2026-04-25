import XCTest
import Foundation
import Darwin

class BooUITestCase: XCTestCase {
    func isConnectScreen(_ app: XCUIApplication) -> Bool {
        let screen = app.otherElements["connect-screen"]
        let hostField = app.textFields["connect-host-input"]
        let connectButton = app.buttons["connect-button"]
        return screen.exists || (hostField.exists && connectButton.exists)
    }

    private var fileConfiguredHostAndPort: (host: String, port: UInt16)? {
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

    var explicitHost: String? {
        ProcessInfo.processInfo.environment["BOO_UI_TEST_HOST"]
            ?? (Bundle.main.infoDictionary?["BOO_UI_TEST_HOST"] as? String)
            ?? fileConfiguredHostAndPort?.host
            ?? GeneratedUITestConfig.host
    }

    var port: UInt16 {
        ProcessInfo.processInfo.environment["BOO_UI_TEST_PORT"].flatMap(UInt16.init)
            ?? (Bundle.main.infoDictionary?["BOO_UI_TEST_PORT"] as? String).flatMap(UInt16.init)
            ?? (Bundle.main.infoDictionary?["BOO_UI_TEST_PORT"] as? NSNumber).map(\.uint16Value)
            ?? fileConfiguredHostAndPort?.port
            ?? GeneratedUITestConfig.port
    }

    func assertDaemonReachableIfConfigured(file: StaticString = #filePath, line: UInt = #line) {
        guard let host = explicitHost else { return }
        XCTAssertTrue(canConnect(host: host, port: port), "Expected UI-test Boo daemon at \(host):\(port)", file: file, line: line)
    }

    @discardableResult
    func installSystemAlertHandler(for app: XCUIApplication) -> NSObjectProtocol {
        addUIInterruptionMonitor(withDescription: "System Alerts") { alert in
            let allowButtons = ["Allow", "OK", "Continue", "Join", "Don’t Allow", "Don't Allow"]
            for label in allowButtons {
                if alert.buttons[label].exists, label != "Don’t Allow", label != "Don't Allow" {
                    alert.buttons[label].tap()
                    return true
                }
            }
            return false
        }
    }

    private func canConnect(host: String, port: UInt16) -> Bool {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { return false }
        defer { close(fd) }

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        _ = host.withCString { inet_pton(AF_INET, $0, &address.sin_addr) }

        return withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) == 0
            }
        }
    }

    func makeApp(
        autoConnect: Bool = false,
        resetStorage: Bool = true,
        mockTailscaleDevices: String? = nil,
        tailscaleToken: String? = nil,
        tailscalePort: UInt16? = nil,
        includeConfiguredHost: Bool = true,
        forcedTerminalErrorKind: String? = nil,
        traceActions: String? = nil,
        traceInputCommand: String? = nil,
        traceOutputMarker: String? = nil,
        targetViewedTabIndex: Int? = nil
    ) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-ApplePersistenceIgnoreState", "YES", "--boo-ui-test-mode"]
        app.launchEnvironment["BOO_UI_TEST_MODE"] = "1"
        if resetStorage {
            app.launchArguments.append("--boo-ui-test-reset-storage")
        }
        if includeConfiguredHost {
            app.launchArguments.append("--boo-ui-test-node-name=Local Boo")
        }
        if includeConfiguredHost, let explicitHost {
            app.launchArguments.append("--boo-ui-test-host=\(explicitHost)")
            app.launchArguments.append("--boo-ui-test-port=\(port)")
            app.launchEnvironment["BOO_UI_TEST_HOST"] = explicitHost
            app.launchEnvironment["BOO_UI_TEST_PORT"] = "\(port)"
        }
        if autoConnect {
            app.launchArguments.append("--boo-ui-test-auto-connect")
        }
        if let mockTailscaleDevices {
            app.launchArguments.append("--boo-ui-test-tailscale-devices=\(mockTailscaleDevices)")
        }
        if let tailscaleToken {
            app.launchArguments.append("--boo-ui-test-tailscale-token=\(tailscaleToken)")
        }
        if let tailscalePort {
            app.launchArguments.append("--boo-ui-test-tailscale-port=\(tailscalePort)")
        }
        if let forcedTerminalErrorKind {
            app.launchArguments.append("--boo-ui-test-terminal-error=\(forcedTerminalErrorKind)")
        }
        if let traceActions {
            app.launchArguments.append("--boo-ui-test-trace-actions=\(traceActions)")
        }
        if let traceInputCommand {
            app.launchArguments.append("--boo-ui-test-trace-input-command=\(traceInputCommand)")
        }
        if let traceOutputMarker {
            app.launchArguments.append("--boo-ui-test-trace-output-marker=\(traceOutputMarker)")
        }
        if let targetViewedTabIndex {
            app.launchArguments.append("--boo-ui-test-target-viewed-tab-index=\(targetViewedTabIndex)")
        }
        app.launchEnvironment["BOO_UI_TEST_AUTO_CONNECT"] = autoConnect ? "1" : "0"
        return app
    }

    func keyboardDismissButton(in app: XCUIApplication) -> XCUIElement {
        let predicates = [
            NSPredicate(format: "label CONTAINS[c] 'hide keyboard'"),
            NSPredicate(format: "label CONTAINS[c] 'dismiss keyboard'"),
            NSPredicate(format: "identifier CONTAINS[c] 'Hide keyboard'"),
            NSPredicate(format: "identifier CONTAINS[c] 'Dismiss keyboard'")
        ]

        for predicate in predicates {
            let button = app.buttons.matching(predicate).firstMatch
            if button.exists {
                return button
            }
        }
        return app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'keyboard' OR identifier CONTAINS[c] 'keyboard'")).firstMatch
    }

    func discoveredDaemonRows(in app: XCUIApplication) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "discovered-daemon-"))
    }

    func firstHittableDiscoveredDaemonRow(in app: XCUIApplication) -> XCUIElement? {
        discoveredDaemonRows(in: app).allElementsBoundByIndex.first(where: \.isHittable)
    }

    func connectToConfiguredBoo(from app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        let connectButton = app.buttons["connect-button"]
        if connectButton.waitForExistence(timeout: 2), connectButton.isEnabled {
            connectButton.tap()
            return
        }
        if app.buttons["saved-node-Local Boo"].waitForExistence(timeout: 2) {
            app.buttons["saved-node-Local Boo"].tap()
            return
        }
        if let explicitHost, app.buttons[explicitHost].waitForExistence(timeout: 1) {
            app.buttons[explicitHost].tap()
            return
        }
        if let explicitHost {
            let hostField = app.textFields["connect-host-input"]
            if hostField.waitForExistence(timeout: 2) {
                hostField.tap()
                hostField.typeText("\(explicitHost):\(port)")
                let manualConnectButton = app.buttons["connect-button"]
                XCTAssertTrue(manualConnectButton.waitForExistence(timeout: 5), file: file, line: line)
                manualConnectButton.tap()
                return
            }
        }
        let discoveredRows = discoveredDaemonRows(in: app)
        if discoveredRows.firstMatch.waitForExistence(timeout: 2),
           let hittableDiscoveredRow = firstHittableDiscoveredDaemonRow(in: app) {
            hittableDiscoveredRow.tap()
            return
        }
        if app.buttons["Local Boo"].waitForExistence(timeout: 1) {
            app.buttons["Local Boo"].tap()
            return
        }
        guard let explicitHost else {
            XCTFail("Expected explicit host or saved Local Boo node for UI test", file: file, line: line)
            return
        }
        let hostField = app.textFields["connect-host-input"]
        XCTAssertTrue(hostField.waitForExistence(timeout: 5), file: file, line: line)
        hostField.tap()
        XCTAssertTrue(hostField.waitForExistence(timeout: 5), file: file, line: line)
        hostField.typeText("\(explicitHost):\(port)")
        let manualConnectButton = app.buttons["connect-button"]
        XCTAssertTrue(manualConnectButton.waitForExistence(timeout: 5), file: file, line: line)
        manualConnectButton.tap()
    }

    func scrollUntilHittable(_ element: XCUIElement, in app: XCUIApplication, maxSwipes: Int = 6, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<maxSwipes {
            if element.isHittable {
                return
            }
            app.swipeUp()
        }
        XCTAssertTrue(element.isHittable, "Element was not hittable after scrolling", file: file, line: line)
    }

    func scrollUntilExists(_ element: XCUIElement, in app: XCUIApplication, maxSwipes: Int = 6, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<maxSwipes {
            if element.exists {
                return
            }
            app.swipeUp()
        }
        XCTAssertTrue(element.exists, "Element did not appear after scrolling", file: file, line: line)
    }

    func navigateToConnectScreen(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if isConnectScreen(app) {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTFail("expected connect screen", file: file, line: line)
    }

    func waitForAnyTailscaleResult(in app: XCUIApplication, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
        let deadline = Date().addingTimeInterval(timeout)
        let peerButtons = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "tailscale-peer-"))
        let errorTexts = app.staticTexts.matching(NSPredicate(format: "label CONTAINS[c] %@", "Tailscale API"))

        while Date() < deadline {
            if peerButtons.count > 0 || errorTexts.count > 0 {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTAssertTrue(peerButtons.count > 0 || errorTexts.count > 0, "Expected Tailscale devices or an API error to appear", file: file, line: line)
    }

    @discardableResult
    func openLiveTerminal(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) -> Bool {
        if isConnectScreen(app) {
            connectToConfiguredBoo(from: app, file: file, line: line)
        }

        let terminal = app.otherElements["terminal-screen"]
        let deadline = Date().addingTimeInterval(12)
        while Date() < deadline {
            if terminal.exists {
                break
            }
            let errorLabel = app.staticTexts["connect-error-label"]
            if errorLabel.exists, !errorLabel.label.isEmpty {
                XCTFail("connect did not reach terminal: \(errorLabel.label)", file: file, line: line)
                return false
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }
        XCTAssertTrue(terminal.exists, "expected terminal after connect", file: file, line: line)
        guard terminal.exists else {
            return false
        }
        let activeExpectation = NSPredicate(format: "label BEGINSWITH %@", "active-")
        let activeResult = XCTWaiter.wait(
            for: [XCTNSPredicateExpectation(predicate: activeExpectation, object: terminal)],
            timeout: 10
        )
        XCTAssertEqual(activeResult, .completed, "terminal did not become active", file: file, line: line)
        guard activeResult == .completed else {
            return false
        }

        let errorTexts = [
            "unreachable",
            "Connection lost",
            "Remote heartbeat timed out",
            "timed out"
        ]
        for text in errorTexts {
            XCTAssertFalse(app.staticTexts.containing(NSPredicate(format: "label CONTAINS[c] %@", text)).firstMatch.exists, "terminal opened in bad state containing '\(text)'", file: file, line: line)
        }
        return true
    }

}
