import XCTest

final class BooAppLaunchTests: BooUITestCase {
    func testConnectScreenShowsMockTailscaleDevices() {
        let mockDevices = "Mac mini|mini.tailnet.ts.net|100.64.0.10|macOS|1;Offline box|offline.ts.net|100.64.0.11|Linux|0"
        let app = makeApp(autoConnect: false, resetStorage: true, mockTailscaleDevices: mockDevices)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)

        let tailscaleSection = app.staticTexts["TAILSCALE DEVICES"]
        scrollUntilExists(tailscaleSection, in: app)

        let macMini = app.buttons["tailscale-peer-Mac mini"]
        let offlineBox = app.buttons["tailscale-peer-Offline box"]
        scrollUntilExists(macMini, in: app)
        scrollUntilExists(offlineBox, in: app)
    }

    func testConnectScreenShowsTailscaleSectionWithoutToken() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)

        let tailscaleSection = app.staticTexts["TAILSCALE DEVICES"]
        scrollUntilExists(tailscaleSection, in: app)

        let missingLabel = app.staticTexts["tailscale-token-missing-label"]
        scrollUntilExists(missingLabel, in: app)
    }

    func testTailscaleTokenCanBeSavedAndCleared() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let settingsButton = app.buttons["settings-button"].exists
            ? app.buttons["settings-button"]
            : app.buttons["tab-settings"]
        XCTAssertTrue(settingsButton.waitForExistence(timeout: 5))
        settingsButton.tap()

        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        XCTAssertEqual(title.label, "Settings")

        let portField = app.textFields["settings-tailscale-port-input"]
        XCTAssertTrue(portField.waitForExistence(timeout: 5))
        portField.tap()
        portField.typeText(XCUIKeyboardKey.delete.rawValue + XCUIKeyboardKey.delete.rawValue + XCUIKeyboardKey.delete.rawValue + XCUIKeyboardKey.delete.rawValue)
        portField.typeText("7337")

        let tokenField = app.textFields["settings-tailscale-token-input"]
        XCTAssertTrue(tokenField.waitForExistence(timeout: 5))
        scrollUntilHittable(tokenField, in: app)
        tokenField.tap()
        tokenField.typeText("tskey-test-ui-token")

        let saveButton = app.buttons["save-tailscale-settings-button"]
        XCTAssertTrue(saveButton.waitForExistence(timeout: 5))
        scrollUntilHittable(saveButton, in: app)
        saveButton.tap()

        let savedLabel = app.staticTexts["API access token saved securely in the iOS Keychain."]
        XCTAssertTrue(savedLabel.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["clear-tailscale-token-button"].waitForExistence(timeout: 5))

        app.terminate()

        let relaunched = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: relaunched)
        relaunched.launch()
        relaunched.tap()

        let relaunchedSettings = relaunched.buttons["settings-button"].exists
            ? relaunched.buttons["settings-button"]
            : relaunched.buttons["tab-settings"]
        XCTAssertTrue(relaunchedSettings.waitForExistence(timeout: 5))
        relaunchedSettings.tap()

        let relaunchedTitle = relaunched.staticTexts["screen-title"]
        XCTAssertTrue(relaunchedTitle.waitForExistence(timeout: 5))
        XCTAssertEqual(relaunchedTitle.label, "Settings")

        let persistedLabel = relaunched.staticTexts["API access token saved securely in the iOS Keychain."]
        XCTAssertTrue(persistedLabel.waitForExistence(timeout: 5))

        let sessionsTab = relaunched.buttons["tab-sessions"]
        XCTAssertTrue(sessionsTab.waitForExistence(timeout: 5))
        sessionsTab.tap()
        navigateToConnectScreen(relaunched)
        let tailscaleSection = relaunched.staticTexts["TAILSCALE DEVICES"]
        scrollUntilExists(tailscaleSection, in: relaunched)

        let clearButton = relaunched.buttons["clear-tailscale-token-button"]
        XCTAssertTrue(clearButton.waitForExistence(timeout: 5))
        scrollUntilHittable(clearButton, in: relaunched)
        clearButton.tap()
        XCTAssertFalse(persistedLabel.waitForExistence(timeout: 1))

        relaunched.terminate()

        let cleared = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: cleared)
        cleared.launch()
        cleared.tap()
        let clearedSettings = cleared.buttons["settings-button"].exists
            ? cleared.buttons["settings-button"]
            : cleared.buttons["tab-settings"]
        XCTAssertTrue(clearedSettings.waitForExistence(timeout: 5))
        clearedSettings.tap()
        let clearedTitle = cleared.staticTexts["screen-title"]
        XCTAssertTrue(clearedTitle.waitForExistence(timeout: 5))
        XCTAssertEqual(clearedTitle.label, "Settings")
        XCTAssertFalse(cleared.staticTexts["API access token saved securely in the iOS Keychain."].exists)
        XCTAssertFalse(cleared.buttons["clear-tailscale-token-button"].exists)
    }

    func testLiveTailscaleDevicesAppearWhenTokenIsSaved() throws {
        let env = ProcessInfo.processInfo.environment
        guard let liveToken = env["BOO_IOS_UI_TEST_TAILSCALE_TOKEN"],
              !liveToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            throw XCTSkip("Set BOO_IOS_UI_TEST_TAILSCALE_TOKEN to run the live Tailscale discovery smoke test.")
        }
        let livePort = env["BOO_IOS_UI_TEST_TAILSCALE_PORT"].flatMap(UInt16.init)
        let app = makeApp(
            autoConnect: false,
            resetStorage: true,
            tailscaleToken: liveToken,
            tailscalePort: livePort
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let settingsButton = app.buttons["settings-button"]
        XCTAssertTrue(settingsButton.waitForExistence(timeout: 5))
        settingsButton.tap()

        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        XCTAssertEqual(title.label, "Settings")

        let savedLabel = app.staticTexts["API access token saved securely in the iOS Keychain."]
        XCTAssertTrue(savedLabel.waitForExistence(timeout: 5))

        let sessionsTab = app.buttons["tab-sessions"]
        XCTAssertTrue(sessionsTab.waitForExistence(timeout: 5))
        sessionsTab.tap()

        navigateToConnectScreen(app)

        let tailscaleSection = app.staticTexts["TAILSCALE DEVICES"]
        scrollUntilExists(tailscaleSection, in: app)
        waitForAnyTailscaleResult(in: app, timeout: 12)
    }

    func testConnectScreenShowsDiscoveredDaemon() {
        let app = makeApp(autoConnect: false, includeConfiguredHost: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        if title.label == "Active Sessions" {
            let disconnectButton = app.buttons["sessions-disconnect-button"]
            XCTAssertTrue(disconnectButton.waitForExistence(timeout: 5))
            disconnectButton.tap()
            XCTAssertTrue(title.waitForExistence(timeout: 5))
        }
        XCTAssertEqual(title.label, "Connect to Server")

        let discoveredRows = discoveredDaemonRows(in: app)
        let firstRow = discoveredRows.firstMatch
        if !firstRow.waitForExistence(timeout: 12) {
            let browserError = app.staticTexts["bonjour-error-label"].label
            let attachment = XCTAttachment(screenshot: app.screenshot())
            attachment.name = "discovery-failure"
            attachment.lifetime = .keepAlways
            add(attachment)
            XCTFail("expected discovered daemon row; browserError='\(browserError)'")
        }

        sleep(2)
        XCTAssertEqual(discoveredRows.count, 1, "expected exactly one discovered daemon row after dedupe")
    }

    func testTappingDiscoveredDaemonConnects() {
        let app = makeApp(autoConnect: false, resetStorage: true, includeConfiguredHost: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)
        let title = app.staticTexts["screen-title"]

        let discoveredRows = discoveredDaemonRows(in: app)
        let firstRow = discoveredRows.firstMatch
        XCTAssertTrue(firstRow.waitForExistence(timeout: 12))
        firstRow.tap()

        let deadline = Date().addingTimeInterval(12)
        while Date() < deadline {
            if title.exists, title.label == "Active Sessions" {
                return
            }
            let errorLabel = app.staticTexts["connect-error-label"]
            if errorLabel.exists, !errorLabel.label.isEmpty {
                XCTFail("discovered daemon connect failed: \(errorLabel.label)")
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        let banner = app.staticTexts["connect-status-banner"].label
        XCTFail("discovered daemon tap never left connect screen; status='\(banner)'")
    }

    func testTappingTailscaleDeviceConnects() {
        let mockDevices = "example-mbp|example-mbp.tailnet.ts.net|100.76.250.75|macOS|1"
        let app = makeApp(autoConnect: false, resetStorage: true, mockTailscaleDevices: mockDevices)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)
        let title = app.staticTexts["screen-title"]

        let tailscaleSection = app.staticTexts["TAILSCALE DEVICES"]
        scrollUntilExists(tailscaleSection, in: app)

        let mbpRow = app.buttons["tailscale-peer-example-mbp"]
        scrollUntilExists(mbpRow, in: app)
        mbpRow.tap()

        let deadline = Date().addingTimeInterval(15)
        while Date() < deadline {
            if title.exists, title.label == "Active Sessions" {
                return
            }
            let errorLabel = app.staticTexts["connect-error-label"]
            if errorLabel.exists, !errorLabel.label.isEmpty {
                XCTAssertFalse(errorLabel.label.contains("NoSuchRecord"), "tailscale device should not fail on DNS lookup: \(errorLabel.label)")
                XCTAssertEqual(errorLabel.label, "Connection timed out")
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        let banner = app.staticTexts["connect-status-banner"].label
        XCTFail("tailscale device tap neither connected nor surfaced a timeout; status='\(banner)'")
    }

    func testTapTab1FromActiveSessionsAndType() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))

        if title.label != "Active Sessions", title.label == "Tab 1" || title.label.hasPrefix("Tab ") {
            let sessionsButton = app.buttons["Sessions"]
            XCTAssertTrue(sessionsButton.waitForExistence(timeout: 5))
            sessionsButton.tap()
            XCTAssertTrue(title.waitForExistence(timeout: 5))
        }

        if title.label != "Active Sessions", let explicitHost {
            _ = explicitHost
            connectToConfiguredBoo(from: app)
            XCTAssertTrue(title.waitForExistence(timeout: 20))
        }

        XCTAssertEqual(title.label, "Active Sessions")
        let tab1 = app.buttons["session-row-1"]
        XCTAssertTrue(tab1.waitForExistence(timeout: 10))
        tab1.tap()

        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10))
        terminal.tap()

        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        let proxy = app.textViews["terminal-text-proxy"]
        XCTAssertTrue(proxy.waitForExistence(timeout: 5))
        proxy.typeText("echo BOO_TAB1_TYPED\r")

        let outputExpectation = NSPredicate(format: "value CONTAINS %@", "BOO_TAB1_TYPED")
        expectation(for: outputExpectation, evaluatedWith: terminal)
        waitForExpectations(timeout: 10)
    }

    func testCaptureActiveSessionsScreenshot() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))

        if title.label != "Active Sessions", let explicitHost {
            let hostField = app.textFields["connect-host-input"]
            XCTAssertTrue(hostField.waitForExistence(timeout: 5))
            hostField.tap()
            hostField.typeText("\(explicitHost):\(port)")

            let connectButton = app.buttons["connect-button"]
            XCTAssertTrue(connectButton.waitForExistence(timeout: 5))
            connectButton.tap()
            XCTAssertTrue(title.waitForExistence(timeout: 20))
        }

        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "active-sessions"
        attachment.lifetime = .keepAlways
        add(attachment)
        XCTAssertEqual(title.label, "Active Sessions")
    }

    func testConnectScreenElementsAppear() {
        let app = makeApp()
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        if title.label == "Active Sessions" {
            let disconnectButton = app.buttons["sessions-disconnect-button"]
            XCTAssertTrue(disconnectButton.waitForExistence(timeout: 5))
            disconnectButton.tap()
            XCTAssertTrue(title.waitForExistence(timeout: 5))
        }
        XCTAssertEqual(title.label, "Connect to Server")
        XCTAssertTrue(app.textFields["connect-host-input"].exists)
        XCTAssertTrue(app.buttons["connect-button"].exists)
        XCTAssertTrue(app.buttons["settings-button"].exists)
        if explicitHost != nil {
            XCTAssertTrue(app.buttons["saved-node-Local Boo"].exists)
        }
    }

    func testAutoConnectCanCreateAndAttachSession() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let createButton = app.buttons["create-session-button"]
        if !createButton.waitForExistence(timeout: 5) {
            connectToConfiguredBoo(from: app)
        }

        if !createButton.waitForExistence(timeout: 20) {
            let errorText = app.staticTexts["connect-error-label"].label
            let statusText = app.staticTexts["connect-status-banner"].label
            let attachment = XCTAttachment(screenshot: app.screenshot())
            attachment.name = "connect-failure"
            attachment.lifetime = .keepAlways
            add(attachment)
            XCTFail("connect did not reach sessions; status='\(statusText)' error='\(errorText)'")
            return
        }
        let existingSessions = sessionRows(in: app)
        if existingSessions.count > 0 {
            existingSessions.element(boundBy: 0).tap()
        } else {
            createButton.tap()
        }

        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10))
        let proxy = app.textViews["terminal-text-proxy"]
        XCTAssertTrue(proxy.waitForExistence(timeout: 5))
        proxy.tap()

        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        proxy.typeText("echo BOO_UI_TYPED\r")

        let outputExpectation = NSPredicate(format: "value CONTAINS %@", "BOO_UI_TYPED")
        expectation(for: outputExpectation, evaluatedWith: terminal)
        waitForExpectations(timeout: 10)

        let dismissButton = keyboardDismissButton(in: app)
        XCTAssertTrue(dismissButton.waitForExistence(timeout: 5))
        dismissButton.tap()
        XCTAssertFalse(keyboard.waitForExistence(timeout: 2))
    }
}
