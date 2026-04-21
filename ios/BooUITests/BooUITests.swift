import XCTest

final class BooAppLaunchTests: BooUITestCase {
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
        createButton.tap()

        let terminalInput = app.textFields["terminal-input"]
        XCTAssertTrue(terminalInput.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["terminal-send-button"].exists)
        XCTAssertTrue(app.otherElements["terminal-screen"].exists)
    }
}
