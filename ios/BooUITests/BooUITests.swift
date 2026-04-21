import XCTest

final class BooAppLaunchTests: BooUITestCase {
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
