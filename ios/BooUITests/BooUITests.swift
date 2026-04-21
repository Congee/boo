import XCTest

final class BooAppLaunchTests: BooUITestCase {
    func testTailscaleTokenCanBeSavedAndCleared() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let settingsButton = app.buttons["settings-button"]
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

        let tokenField = app.secureTextFields["settings-tailscale-token-input"]
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

        let relaunchedSettings = relaunched.buttons["settings-button"]
        XCTAssertTrue(relaunchedSettings.waitForExistence(timeout: 5))
        relaunchedSettings.tap()

        let relaunchedTitle = relaunched.staticTexts["screen-title"]
        XCTAssertTrue(relaunchedTitle.waitForExistence(timeout: 5))
        XCTAssertEqual(relaunchedTitle.label, "Settings")

        let persistedLabel = relaunched.staticTexts["API access token saved securely in the iOS Keychain."]
        XCTAssertTrue(persistedLabel.waitForExistence(timeout: 5))

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
        let clearedSettings = cleared.buttons["settings-button"]
        XCTAssertTrue(clearedSettings.waitForExistence(timeout: 5))
        clearedSettings.tap()
        let clearedTitle = cleared.staticTexts["screen-title"]
        XCTAssertTrue(clearedTitle.waitForExistence(timeout: 5))
        XCTAssertEqual(clearedTitle.label, "Settings")
        XCTAssertFalse(cleared.staticTexts["API access token saved securely in the iOS Keychain."].exists)
        XCTAssertFalse(cleared.buttons["clear-tailscale-token-button"].exists)
    }

    func testConnectScreenShowsDiscoveredDaemon() {
        let app = makeApp(autoConnect: false)
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
        XCTAssertTrue(firstRow.waitForExistence(timeout: 12))
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
            let hostField = app.textFields["connect-host-input"]
            XCTAssertTrue(hostField.waitForExistence(timeout: 5))
            hostField.tap()
            hostField.typeText("\(explicitHost):\(port)")

            let connectButton = app.buttons["connect-button"]
            XCTAssertTrue(connectButton.waitForExistence(timeout: 5))
            connectButton.tap()
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
        XCTAssertNotNil(explicitHost)
        let app = makeApp(autoConnect: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let createButton = app.buttons["create-session-button"]
        if !createButton.waitForExistence(timeout: 5) {
            let hostField = app.textFields["connect-host-input"]
            XCTAssertTrue(hostField.waitForExistence(timeout: 5))
            hostField.tap()
            hostField.typeText("\(explicitHost!):\(port)")

            let connectButton = app.buttons["connect-button"]
            XCTAssertTrue(connectButton.waitForExistence(timeout: 5))
            connectButton.tap()
        }

        XCTAssertTrue(createButton.waitForExistence(timeout: 20))
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
