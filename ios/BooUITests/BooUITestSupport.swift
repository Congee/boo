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

    func makeApp(autoConnect: Bool = false, resetStorage: Bool = true) -> XCUIApplication {
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
        app.launchEnvironment["BOO_UI_TEST_AUTO_CONNECT"] = autoConnect ? "1" : "0"
        return app
    }
}
