import XCTest

final class BooAppLaunchTests: BooUITestCase {
    private func uiStateSnapshot(_ app: XCUIApplication) -> String {
        let title = app.staticTexts["screen-title"].exists ? app.staticTexts["screen-title"].label : "<none>"
        let connectStatus = app.staticTexts["connect-status-banner"].exists ? app.staticTexts["connect-status-banner"].label : "<none>"
        let connectError = app.staticTexts["connect-error-label"].exists ? app.staticTexts["connect-error-label"].label : "<none>"
        let bonjourError = app.staticTexts["bonjour-error-label"].exists ? app.staticTexts["bonjour-error-label"].label : "<none>"
        let terminalBanner = app.staticTexts["terminal-banner-label"].exists ? app.staticTexts["terminal-banner-label"].label : "<none>"
        let terminal = app.otherElements["terminal-screen"]
        let terminalExists = terminal.exists
        let terminalLabel = terminalExists ? terminal.label : "<none>"
        let terminalValue = terminalExists ? String(describing: terminal.value ?? "<nil>") : "<none>"
        let floatingBack = app.buttons["floating-back-button"].exists
        return """
        title=\(title)
        connectStatus=\(connectStatus)
        connectError=\(connectError)
        bonjourError=\(bonjourError)
        terminalBanner=\(terminalBanner)
        terminalExists=\(terminalExists)
        terminalLabel=\(terminalLabel)
        terminalValuePrefix=\(terminalValue.prefix(200))
        floatingBack=\(floatingBack)
        """
    }

    private func attachStateSnapshot(_ app: XCUIApplication) -> String {
        let terminal = app.otherElements["terminal-screen"]
        let label = terminal.exists ? terminal.label : "<no terminal>"
        let value = terminal.exists ? String(describing: terminal.value ?? "<nil>") : "<no terminal>"
        return "terminalLabel=\(label) terminalValuePrefix=\(value.prefix(200))"
    }

    private func waitForConnectScreen(_ app: XCUIApplication, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
        let title = app.staticTexts["screen-title"]
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if title.exists, isConnectScreenTitle(title.label) {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "connect-screen-timeout"
        attachment.lifetime = .keepAlways
        add(attachment)
        XCTFail("expected connect screen, got:\n\(uiStateSnapshot(app))", file: file, line: line)
    }

    private func waitForTerminalScreen(_ app: XCUIApplication, timeout: TimeInterval = 12, file: StaticString = #filePath, line: UInt = #line) {
        let terminal = app.otherElements["terminal-screen"]
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if terminal.exists {
                return
            }
            let errorLabel = app.staticTexts["connect-error-label"]
            if errorLabel.exists, !errorLabel.label.isEmpty {
                let attachment = XCTAttachment(screenshot: app.screenshot())
                attachment.name = "terminal-connect-error"
                attachment.lifetime = .keepAlways
                add(attachment)
                XCTFail("connect did not reach terminal: \(errorLabel.label)\n\(uiStateSnapshot(app))", file: file, line: line)
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "terminal-timeout"
        attachment.lifetime = .keepAlways
        add(attachment)
        XCTFail("expected terminal screen, got:\n\(uiStateSnapshot(app))", file: file, line: line)
    }

    private func assertTerminalCanType(_ app: XCUIApplication, marker: String, file: StaticString = #filePath, line: UInt = #line) {
        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10), file: file, line: line)
        terminal.tap()

        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5), file: file, line: line)
        let proxy = app.textViews["terminal-text-proxy"]
        XCTAssertTrue(proxy.waitForExistence(timeout: 5), file: file, line: line)
        proxy.tap()
        proxy.typeText("echo \(marker)\r")

        let outputExpectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value CONTAINS %@", marker),
            object: terminal
        )
        let result = XCTWaiter.wait(for: [outputExpectation], timeout: 10)
        XCTAssertEqual(
            result,
            .completed,
            "typed text did not appear in terminal: \(attachStateSnapshot(app))\n\(uiStateSnapshot(app))",
            file: file,
            line: line
        )

        let attachedPrefix = NSPredicate(format: "label BEGINSWITH %@", "attached-")
        XCTAssertTrue(attachedPrefix.evaluate(with: terminal), "terminal lost attachment after typing: \(attachStateSnapshot(app))", file: file, line: line)
    }

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
        XCTAssertTrue(isConnectScreenTitle(title.label))

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

        let discoveredRows = discoveredDaemonRows(in: app)
        let firstRow = discoveredRows.firstMatch
        XCTAssertTrue(firstRow.waitForExistence(timeout: 12))
        firstRow.tap()

        let deadline = Date().addingTimeInterval(12)
        while Date() < deadline {
            if app.otherElements["terminal-screen"].exists {
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

        let tailscaleSection = app.staticTexts["TAILSCALE DEVICES"]
        scrollUntilExists(tailscaleSection, in: app)

        let mbpRow = app.buttons["tailscale-peer-example-mbp"]
        scrollUntilExists(mbpRow, in: app)
        mbpRow.tap()

        let deadline = Date().addingTimeInterval(15)
        while Date() < deadline {
            if app.otherElements["terminal-screen"].exists {
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

    func testOpenLiveSessionAndType() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }
        assertTerminalCanType(app, marker: "BOO_UI_TYPED")
    }

    func testSwipeBackFromTerminalReturnsToConnectScreen() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10))
        let backZone = app.otherElements["terminal-back-swipe-zone"]
        XCTAssertTrue(backZone.waitForExistence(timeout: 5))
        let start = backZone.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5))
        let finish = app.coordinate(withNormalizedOffset: CGVector(dx: 0.75, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: finish)

        let title = app.staticTexts["screen-title"]
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if title.exists, isConnectScreenTitle(title.label) {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTFail("expected swipe-back to return to the connect screen")
    }

    func testFloatingBackButtonReturnsToConnectScreen() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let floatingBackButton = app.buttons["floating-back-button"]
        XCTAssertTrue(floatingBackButton.waitForExistence(timeout: 5))
        floatingBackButton.tap()

        let title = app.staticTexts["screen-title"]
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if title.exists, isConnectScreenTitle(title.label) {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTFail("expected floating back button to return to the connect screen")
    }

    func testReconnectAndTypeAgainAfterBackNavigation() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }
        assertTerminalCanType(app, marker: "BOO_UI_TYPED_1")

        let floatingBackButton = app.buttons["floating-back-button"]
        XCTAssertTrue(floatingBackButton.waitForExistence(timeout: 5))
        floatingBackButton.tap()

        waitForConnectScreen(app)

        guard openLiveTerminal(app) else { return }
        waitForTerminalScreen(app)
        assertTerminalCanType(app, marker: "BOO_UI_TYPED_2")
    }

    func testKeyboardDismissAndRefocusStillTypes() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let terminal = app.otherElements["terminal-screen"]
        terminal.tap()

        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        let dismissButton = keyboardDismissButton(in: app)
        XCTAssertTrue(dismissButton.waitForExistence(timeout: 5))
        dismissButton.tap()
        XCTAssertFalse(keyboard.waitForExistence(timeout: 2))

        assertTerminalCanType(app, marker: "BOO_UI_TYPED_REFOCUS")
    }

    func testNewSessionRecoveryActionStillTypes() {
        let app = makeApp(
            autoConnect: false,
            resetStorage: false,
            forcedTerminalErrorKind: "attachmentResumeWindowExpired"
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let terminal = app.otherElements["terminal-screen"]
        let originalLabel = terminal.label

        let newSessionButton = app.buttons["new-session-button"]
        XCTAssertTrue(newSessionButton.waitForExistence(timeout: 5))
        newSessionButton.tap()

        let labelChange = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label != %@", originalLabel),
            object: terminal
        )
        XCTAssertEqual(XCTWaiter.wait(for: [labelChange], timeout: 10), .completed)

        assertTerminalCanType(app, marker: "BOO_UI_TYPED_NEW")
    }

    func testCloseSessionRecoveryActionStillTypes() {
        let app = makeApp(
            autoConnect: false,
            resetStorage: false,
            forcedTerminalErrorKind: "attachmentResumeWindowExpired"
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let terminal = app.otherElements["terminal-screen"]
        let firstLabel = terminal.label

        let newSessionButton = app.buttons["new-session-button"]
        XCTAssertTrue(newSessionButton.waitForExistence(timeout: 5))
        newSessionButton.tap()

        let labelChange = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label != %@", firstLabel),
            object: terminal
        )
        XCTAssertEqual(XCTWaiter.wait(for: [labelChange], timeout: 10), .completed)
        let secondLabel = terminal.label

        let closeSessionButton = app.buttons["close-session-button"]
        XCTAssertTrue(closeSessionButton.waitForExistence(timeout: 5))
        closeSessionButton.tap()

        let relabeled = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label != %@", secondLabel),
            object: terminal
        )
        XCTAssertEqual(XCTWaiter.wait(for: [relabeled], timeout: 10), .completed)

        assertTerminalCanType(app, marker: "BOO_UI_TYPED_CLOSE")
    }

    func testConnectScreenElementsAppear() {
        let app = makeApp()
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        XCTAssertTrue(isConnectScreenTitle(title.label))
        XCTAssertTrue(app.textFields["connect-host-input"].exists)
        XCTAssertTrue(app.buttons["connect-button"].exists)
        XCTAssertTrue(app.buttons["settings-button"].exists)
    }

    func testAutoConnectCanCreateAndAttachSession() {
        let app = makeApp(autoConnect: false, resetStorage: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        openLiveTerminal(app)
        assertTerminalCanType(app, marker: "BOO_UI_TYPED")

        let keyboard = app.keyboards.firstMatch
        let dismissButton = keyboardDismissButton(in: app)
        XCTAssertTrue(dismissButton.waitForExistence(timeout: 5))
        dismissButton.tap()
        XCTAssertFalse(keyboard.waitForExistence(timeout: 2))
    }
}
