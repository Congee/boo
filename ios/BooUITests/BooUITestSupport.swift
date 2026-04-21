import XCTest
import Foundation
import Darwin

class BooUITestCase: XCTestCase {
    var explicitHost: String? {
        ProcessInfo.processInfo.environment["BOO_UI_TEST_HOST"]
            ?? (Bundle.main.infoDictionary?["BOO_UI_TEST_HOST"] as? String)
            ?? GeneratedUITestConfig.host
    }

    var port: UInt16 {
        ProcessInfo.processInfo.environment["BOO_UI_TEST_PORT"].flatMap(UInt16.init)
            ?? (Bundle.main.infoDictionary?["BOO_UI_TEST_PORT"] as? String).flatMap(UInt16.init)
            ?? (Bundle.main.infoDictionary?["BOO_UI_TEST_PORT"] as? NSNumber).map(\.uint16Value)
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

    func makeApp(autoConnect: Bool = false, resetStorage: Bool = true, mockTailscaleDevices: String? = nil) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-ApplePersistenceIgnoreState", "YES", "--boo-ui-test-mode"]
        app.launchEnvironment["BOO_UI_TEST_MODE"] = "1"
        if resetStorage {
            app.launchArguments.append("--boo-ui-test-reset-storage")
        }
        app.launchArguments.append("--boo-ui-test-node-name=Local Boo")
        if let explicitHost {
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
        app.launchEnvironment["BOO_UI_TEST_AUTO_CONNECT"] = autoConnect ? "1" : "0"
        return app
    }

    func sessionRows(in app: XCUIApplication) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "session-row-"))
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
        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5), file: file, line: line)
        if title.label == "Active Sessions" {
            let disconnectButton = app.buttons["sessions-disconnect-button"]
            XCTAssertTrue(disconnectButton.waitForExistence(timeout: 5), file: file, line: line)
            disconnectButton.tap()
            XCTAssertTrue(title.waitForExistence(timeout: 5), file: file, line: line)
        }
        XCTAssertEqual(title.label, "Connect to Server", file: file, line: line)
    }
}
