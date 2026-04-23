import XCTest

final class BooAppLaunchTests: BooUITestCase {
    private func uiStateSnapshot(_ app: XCUIApplication) -> String {
        let title = app.staticTexts["screen-title"].exists ? app.staticTexts["screen-title"].label : "<none>"
        let connectScreen = app.otherElements["connect-screen"].exists
        let connectStatus = app.staticTexts["connect-status-banner"].exists ? app.staticTexts["connect-status-banner"].label : "<none>"
        let connectError = app.staticTexts["connect-error-label"].exists ? app.staticTexts["connect-error-label"].label : "<none>"
        let bonjourError = app.staticTexts["bonjour-error-label"].exists ? app.staticTexts["bonjour-error-label"].label : "<none>"
        let connectHostExists = app.textFields["connect-host-input"].exists
        let connectHostValue = connectHostExists ? String(describing: app.textFields["connect-host-input"].value ?? "<nil>") : "<none>"
        let terminalBanner = app.staticTexts["terminal-banner-label"].exists ? app.staticTexts["terminal-banner-label"].label : "<none>"
        let terminalDebug = app.otherElements["terminal-debug-state"].exists ? app.otherElements["terminal-debug-state"].label : "<none>"
        let seedStatus = app.staticTexts["uitest-scroll-seed-status"].exists ? app.staticTexts["uitest-scroll-seed-status"].label : "<none>"
        let terminal = app.otherElements["terminal-screen"]
        let terminalExists = terminal.exists
        let terminalLabel = terminalExists ? terminal.label : "<none>"
        let terminalValue = terminalExists ? String(describing: terminal.value ?? "<nil>") : "<none>"
        let floatingBack = app.buttons["floating-back-button"].exists
        return """
        title=\(title)
        connectScreen=\(connectScreen)
        connectStatus=\(connectStatus)
        connectError=\(connectError)
        bonjourError=\(bonjourError)
        connectHostExists=\(connectHostExists)
        connectHostValue=\(connectHostValue)
        terminalBanner=\(terminalBanner)
        terminalDebug=\(terminalDebug)
        seedStatus=\(seedStatus)
        terminalExists=\(terminalExists)
        terminalLabel=\(terminalLabel)
        terminalValuePrefix=\(terminalValue.prefix(200))
        floatingBack=\(floatingBack)
        """
    }

    private func activeTabStateSnapshot(_ app: XCUIApplication) -> String {
        let terminal = app.otherElements["terminal-screen"]
        let label = terminal.exists ? terminal.label : "<no terminal>"
        let value = terminal.exists ? String(describing: terminal.value ?? "<nil>") : "<no terminal>"
        return "terminalLabel=\(label) terminalValuePrefix=\(value.prefix(200))"
    }

    private func waitForConnectScreen(_ app: XCUIApplication, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
        let connectButton = app.buttons["connect-button"]
        let savedNode = app.buttons["saved-node-Local Boo"]
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            let hasHittableConnectAction =
                connectButton.isHittable ||
                savedNode.isHittable ||
                firstHittableDiscoveredDaemonRow(in: app) != nil
            if isConnectScreen(app), hasHittableConnectAction {
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

        let activeExpectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label BEGINSWITH %@", "active-"),
            object: terminal
        )
        let activeResult = XCTWaiter.wait(for: [activeExpectation], timeout: 10)
        XCTAssertEqual(
            activeResult,
            .completed,
            "terminal never reached active state before typing: \(activeTabStateSnapshot(app))\n\(uiStateSnapshot(app))",
            file: file,
            line: line
        )

        let keyboard = app.keyboards.firstMatch
        let proxy = app.textViews["terminal-text-proxy"]
        XCTAssertTrue(proxy.waitForExistence(timeout: 5), file: file, line: line)
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5), file: file, line: line)

        var typedSuccessfully = false
        for _ in 0..<2 {
            proxy.tap()
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
            proxy.typeText("echo \(marker)\r")

            let outputExpectation = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "value CONTAINS %@", marker),
                object: terminal
            )
            if XCTWaiter.wait(for: [outputExpectation], timeout: 6) == .completed {
                typedSuccessfully = true
                break
            }
        }

        XCTAssertTrue(
            typedSuccessfully,
            "typed text did not appear in terminal: \(activeTabStateSnapshot(app))\n\(uiStateSnapshot(app))",
            file: file,
            line: line
        )
        XCTAssertTrue(activeExpectation.predicate.evaluate(with: terminal), "terminal lost active tab after typing: \(activeTabStateSnapshot(app))", file: file, line: line)
    }

    private func dragTerminal(_ terminal: XCUIElement, upward: Bool) {
        let startY: CGFloat = upward ? 0.78 : 0.28
        let endY: CGFloat = upward ? 0.22 : 0.82
        let start = terminal.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: startY))
        let finish = terminal.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: endY))
        start.press(forDuration: 0.05, thenDragTo: finish)
    }

    private func visibleScrollMarkers(in value: String) -> [Int] {
        let pattern = /BOO_SCROLL_(\d+)/
        return value.matches(of: pattern).compactMap { Int($0.output.1) }
    }

    private func sendTerminalCommand(_ app: XCUIApplication, command: String, expect marker: String, timeout: TimeInterval = 8, file: StaticString = #filePath, line: UInt = #line) {
        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10), file: file, line: line)

        let proxy = app.textViews["terminal-text-proxy"]
        XCTAssertTrue(proxy.waitForExistence(timeout: 5), file: file, line: line)
        var commandObserved = false
        for _ in 0..<2 {
            proxy.tap()
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
            proxy.typeText(command + "\r")

            let expectation = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "value CONTAINS %@", marker),
                object: terminal
            )
            if XCTWaiter.wait(for: [expectation], timeout: timeout) == .completed {
                commandObserved = true
                break
            }
        }

        XCTAssertTrue(
            commandObserved,
            "terminal never printed '\(marker)' after command '\(command)': \(uiStateSnapshot(app))",
            file: file,
            line: line
        )
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

        let settingsButton = app.buttons["tab-settings"]
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

        let relaunchedSettings = relaunched.buttons["tab-settings"]
        XCTAssertTrue(relaunchedSettings.waitForExistence(timeout: 5))
        relaunchedSettings.tap()

        let relaunchedTitle = relaunched.staticTexts["screen-title"]
        XCTAssertTrue(relaunchedTitle.waitForExistence(timeout: 5))
        XCTAssertEqual(relaunchedTitle.label, "Settings")

        let persistedLabel = relaunched.staticTexts["API access token saved securely in the iOS Keychain."]
        XCTAssertTrue(persistedLabel.waitForExistence(timeout: 5))

        let terminalTab = relaunched.buttons["tab-terminal"]
        XCTAssertTrue(terminalTab.waitForExistence(timeout: 5))
        terminalTab.tap()
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
        let clearedSettings = cleared.buttons["tab-settings"]
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

        let settingsButton = app.buttons["tab-settings"]
        XCTAssertTrue(settingsButton.waitForExistence(timeout: 5))
        settingsButton.tap()

        let title = app.staticTexts["screen-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        XCTAssertEqual(title.label, "Settings")

        let savedLabel = app.staticTexts["API access token saved securely in the iOS Keychain."]
        XCTAssertTrue(savedLabel.waitForExistence(timeout: 5))

        let terminalTab = app.buttons["tab-terminal"]
        XCTAssertTrue(terminalTab.waitForExistence(timeout: 5))
        terminalTab.tap()

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

        navigateToConnectScreen(app)

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

    func testTappingDiscoveredDaemonConnectsAndTypes() {
        let app = makeApp(autoConnect: false, resetStorage: true, includeConfiguredHost: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)

        let discoveredRows = discoveredDaemonRows(in: app)
        let firstRow = discoveredRows.firstMatch
        XCTAssertTrue(firstRow.waitForExistence(timeout: 12))
        firstRow.tap()

        waitForTerminalScreen(app)
        assertTerminalCanType(app, marker: "BOO_DISCOVERED_TYPED")
    }

    func testFloatingDisconnectButtonClosesHostTab() {
        let app = makeApp(autoConnect: false, resetStorage: true, includeConfiguredHost: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)

        let discoveredRows = discoveredDaemonRows(in: app)
        let firstRow = discoveredRows.firstMatch
        XCTAssertTrue(firstRow.waitForExistence(timeout: 12))
        firstRow.tap()

        waitForTerminalScreen(app)

        let disconnectButton = app.buttons["floating-disconnect-button"]
        XCTAssertTrue(disconnectButton.waitForExistence(timeout: 5))
        disconnectButton.tap()

        waitForConnectScreen(app)
        XCTAssertFalse(app.otherElements["terminal-screen"].exists)
    }

    func testClosingHostTabAllowsFreshDiscoveredReconnect() {
        let app = makeApp(autoConnect: false, resetStorage: true, includeConfiguredHost: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)

        let discoveredRows = discoveredDaemonRows(in: app)
        let firstRow = discoveredRows.firstMatch
        XCTAssertTrue(firstRow.waitForExistence(timeout: 12))
        firstRow.tap()

        waitForTerminalScreen(app)
        assertTerminalCanType(app, marker: "BOO_HOST_SESSION_ONE")

        let disconnectButton = app.buttons["floating-disconnect-button"]
        XCTAssertTrue(disconnectButton.waitForExistence(timeout: 5))
        disconnectButton.tap()

        waitForConnectScreen(app)
        XCTAssertTrue(firstRow.waitForExistence(timeout: 12))
        firstRow.tap()

        waitForTerminalScreen(app)
        assertTerminalCanType(app, marker: "BOO_HOST_SESSION_TWO")
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

    func testOpenLiveTabAndType() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }
        assertTerminalCanType(app, marker: "BOO_UI_TYPED")
    }

    func testOpenLiveTabShowsCustomKeyboardAccessory() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10))
        terminal.tap()

        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))

        XCTAssertTrue(app.buttons["terminal-key-ctrl"].exists)
        XCTAssertTrue(app.buttons["terminal-key-alt"].exists)
        XCTAssertTrue(app.buttons["terminal-key-tab"].exists)
        XCTAssertTrue(app.buttons["terminal-key-tilde"].exists)
        XCTAssertTrue(app.buttons["terminal-key-dollar"].exists)
        XCTAssertTrue(app.buttons["terminal-key-backslash"].exists)
        XCTAssertTrue(app.buttons["terminal-key-left-bracket"].exists)
        XCTAssertTrue(app.buttons["terminal-key-right-bracket"].exists)
        XCTAssertTrue(app.buttons["terminal-key-less-than"].exists)
        XCTAssertTrue(app.buttons["terminal-key-greater-than"].exists)
        XCTAssertTrue(app.buttons["terminal-key-left"].exists)
        XCTAssertTrue(app.buttons["terminal-key-right"].exists)
        XCTAssertTrue(app.buttons["terminal-key-meta"].exists)
    }

    func testKeyboardAccessoryCtrlLClearsVisibleTerminal() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10))
        let proxy = app.textViews["terminal-text-proxy"]
        XCTAssertTrue(proxy.waitForExistence(timeout: 5))
        assertTerminalCanType(app, marker: "CTRL_CLEAR_MARKER")

        proxy.tap()
        proxy.typeKey("l", modifierFlags: .control)
        proxy.typeText("echo AFTER_CTRL_L\r")

        let afterExpectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value CONTAINS %@", "AFTER_CTRL_L"),
            object: terminal
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [afterExpectation], timeout: 10),
            .completed,
            "terminal never recovered after Ctrl+L: \(uiStateSnapshot(app))"
        )
        XCTAssertFalse(
            (terminal.value as? String ?? "").contains("CTRL_CLEAR_MARKER"),
            "Ctrl+L did not clear the visible terminal snapshot: \(uiStateSnapshot(app))"
        )
    }

    func testKeyboardAccessoryRepeatableKeyRepeats() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10))
        terminal.tap()

        let tildeButton = app.descendants(matching: .any).matching(identifier: "terminal-key-tilde").firstMatch
        XCTAssertTrue(tildeButton.waitForExistence(timeout: 5))
        tildeButton.press(forDuration: 1.2)

        let repeatedExpectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value CONTAINS %@", "~~~"),
            object: terminal
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [repeatedExpectation], timeout: 10),
            .completed,
            "repeatable smart key did not emit repeated characters: \(uiStateSnapshot(app))"
        )
    }

    func testFingerScrollUsesTerminalScrollPath() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let terminal = app.otherElements["terminal-screen"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10))

        assertTerminalCanType(app, marker: "BOO_SCROLL_READY")
        sendTerminalCommand(app, command: "jot -w BOO_SCROLL_%d 80 1", expect: "BOO_SCROLL_80", timeout: 12)

        let initialValue = terminal.value as? String ?? ""
        let initialMarkers = visibleScrollMarkers(in: initialValue)
        let initialLowestVisible = initialMarkers.min()
        XCTAssertNotNil(
            initialLowestVisible,
            "terminal snapshot did not expose any scroll markers before dragging: \(uiStateSnapshot(app))"
        )
        var observedLowestVisible = initialLowestVisible.map { [$0] } ?? []

        let beforeAttachment = XCTAttachment(screenshot: app.screenshot())
        beforeAttachment.name = "terminal-before-scroll"
        beforeAttachment.lifetime = .keepAlways
        add(beforeAttachment)

        for upward in [false, true] {
            for _ in 0..<6 {
                dragTerminal(terminal, upward: upward)
                RunLoop.current.run(until: Date().addingTimeInterval(0.35))
                let value = terminal.value as? String ?? ""
                let currentMarkers = visibleScrollMarkers(in: value)
                if let currentLowestVisible = currentMarkers.min() {
                    observedLowestVisible.append(currentLowestVisible)
                }
            }
        }

        let afterAttachment = XCTAttachment(screenshot: app.screenshot())
        afterAttachment.name = "terminal-after-scroll"
        afterAttachment.lifetime = .keepAlways
        add(afterAttachment)

        let materialMovementObserved: Bool
        if let minObserved = observedLowestVisible.min(), let maxObserved = observedLowestVisible.max() {
            materialMovementObserved = (maxObserved - minObserved) >= 4
        } else {
            materialMovementObserved = false
        }

        XCTAssertTrue(
            materialMovementObserved,
            "finger drag did not move the visible terminal scrollback materially via Boo scroll handling; observed lowest markers: \(observedLowestVisible) \(uiStateSnapshot(app))"
        )
    }

    func testDashboardRowShowsLatencyMetricAfterConnect() {
        let app = makeApp(autoConnect: false, resetStorage: true, includeConfiguredHost: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let floatingBackButton = app.buttons["floating-back-button"]
        XCTAssertTrue(floatingBackButton.waitForExistence(timeout: 5))
        floatingBackButton.tap()

        waitForConnectScreen(app)

        let discoveredRows = discoveredDaemonRows(in: app)
        let firstRow = discoveredRows.firstMatch
        XCTAssertTrue(firstRow.waitForExistence(timeout: 12))

        let metricBadge = app.staticTexts.matching(identifier: "host-metric-example-mbp").firstMatch
        XCTAssertTrue(metricBadge.waitForExistence(timeout: 8), "expected visible discovered-host metric badge")

        let predicate = NSPredicate(format: "label MATCHES %@", "\\b[0-9]+ ms\\b")
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: metricBadge)
        let result = XCTWaiter.wait(for: [expectation], timeout: 8)
        XCTAssertEqual(
            result,
            .completed,
            "expected discovered-host metric badge to contain latency text, got '\(metricBadge.label)'"
        )

        // Let the dashboard sit long enough to catch transient row-state regressions
        // before taking the acceptance screenshot.
        sleep(5)

        let screenshotAttachment = XCTAttachment(screenshot: app.screenshot())
        screenshotAttachment.name = "dashboard-row-metrics"
        screenshotAttachment.lifetime = .keepAlways
        add(screenshotAttachment)
    }

    func testTailscaleRowShowsProbeOrLatencyState() {
        let mockDevices = "blackbox|100.124.214.64|100.124.214.64|linux|1"
        let app = makeApp(
            autoConnect: false,
            resetStorage: true,
            mockTailscaleDevices: mockDevices,
            includeConfiguredHost: false
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)

        let tailscaleSection = app.staticTexts["TAILSCALE DEVICES"]
        scrollUntilExists(tailscaleSection, in: app)

        let row = app.buttons["tailscale-peer-blackbox"].exists ? app.buttons["tailscale-peer-blackbox"] : app.otherElements["tailscale-peer-blackbox"]
        XCTAssertTrue(row.waitForExistence(timeout: 10), "expected visible Tailscale row for blackbox")

        let metricBadge = app.staticTexts.matching(identifier: "host-metric-blackbox").firstMatch
        XCTAssertTrue(metricBadge.waitForExistence(timeout: 10), "expected visible Tailscale metric badge for blackbox")

        let predicate = NSPredicate(format: "label MATCHES %@", "\\b(probing|unreachable|[0-9]+ ms)\\b")
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: metricBadge)
        let result = XCTWaiter.wait(for: [expectation], timeout: 10)
        XCTAssertEqual(
            result,
            .completed,
            "expected visible Tailscale metric badge state, got '\(metricBadge.label)'"
        )

        // Allow probe state to settle before capturing evidence.
        sleep(5)

        let screenshotAttachment = XCTAttachment(screenshot: app.screenshot())
        screenshotAttachment.name = "tailscale-row-metrics"
        screenshotAttachment.lifetime = .keepAlways
        add(screenshotAttachment)
    }

    func testOfflineTailscaleRowIsNotTappable() {
        let mockDevices = "Online Mac|online.tailnet.ts.net|100.64.0.10|macOS|1;Offline box|offline.tailnet.ts.net|100.64.0.11|Linux|0"
        let app = makeApp(autoConnect: false, resetStorage: true, mockTailscaleDevices: mockDevices)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)

        let onlineRow = app.buttons["tailscale-peer-Online Mac"]
        let offlineButtonRow = app.buttons["tailscale-peer-Offline box"]
        let offlineAnyRow = app.descendants(matching: .any).matching(identifier: "tailscale-peer-Offline box").firstMatch

        scrollUntilExists(onlineRow, in: app)
        scrollUntilExists(offlineAnyRow, in: app)

        XCTAssertTrue(onlineRow.exists)
        XCTAssertFalse(offlineButtonRow.exists, "offline Tailscale row should not be rendered as a tappable button")
        XCTAssertTrue(offlineAnyRow.exists, "offline Tailscale row should still be visible")

        let screenshotAttachment = XCTAttachment(screenshot: app.screenshot())
        screenshotAttachment.name = "offline-tailscale-row"
        screenshotAttachment.lifetime = .keepAlways
        add(screenshotAttachment)
    }

    func testLiveDashboardScreenshot() {
        let app = XCUIApplication()
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let terminalTab = app.buttons["tab-terminal"]
        if terminalTab.waitForExistence(timeout: 5) {
            terminalTab.tap()
        }

        navigateToConnectScreen(app)

        // Give live Bonjour/Tailscale rows time to settle before capturing.
        sleep(8)

        let screenshotAttachment = XCTAttachment(screenshot: app.screenshot())
        screenshotAttachment.name = "live-dashboard"
        screenshotAttachment.lifetime = .keepAlways
        add(screenshotAttachment)
    }

    func testSwipeBackFromTerminalReturnsToConnectScreen() {
        let app = makeApp(autoConnect: false, resetStorage: true)
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

        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if isConnectScreen(app) {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTFail("expected swipe-back to return to the connect screen")
    }

    func testFloatingBackButtonReturnsToConnectScreen() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let floatingBackButton = app.buttons["floating-back-button"]
        XCTAssertTrue(floatingBackButton.waitForExistence(timeout: 5))
        floatingBackButton.tap()

        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if isConnectScreen(app) {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTFail("expected floating back button to return to the connect screen")
    }

    func testReconnectAndTypeAgainAfterBackNavigation() {
        let app = makeApp(autoConnect: false, resetStorage: true)
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

    func testFastSwipeBackAndReconnectStress() {
        let app = makeApp(autoConnect: false, resetStorage: true, includeConfiguredHost: false)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        let loops = 5
        for iteration in 1...loops {
            guard openLiveTerminal(app) else {
                XCTFail("failed to open live terminal on iteration \(iteration):\n\(uiStateSnapshot(app))")
                return
            }

            let terminal = app.otherElements["terminal-screen"]
            XCTAssertTrue(terminal.waitForExistence(timeout: 10))
            let backZone = app.otherElements["terminal-back-swipe-zone"]
            XCTAssertTrue(backZone.waitForExistence(timeout: 5))
            let start = backZone.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5))
            let finish = app.coordinate(withNormalizedOffset: CGVector(dx: 0.75, dy: 0.5))
            start.press(forDuration: 0.01, thenDragTo: finish)

            waitForConnectScreen(app)

            let discoveredRows = discoveredDaemonRows(in: app)
            let firstRow = discoveredRows.firstMatch
            XCTAssertTrue(firstRow.waitForExistence(timeout: 12), "missing discovered host row after swipe-back on iteration \(iteration):\n\(uiStateSnapshot(app))")
            firstRow.tap()

            waitForTerminalScreen(app)
            let activeExpectation = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "label BEGINSWITH %@", "active-"),
                object: terminal
            )
            let activeResult = XCTWaiter.wait(for: [activeExpectation], timeout: 10)
            XCTAssertEqual(
                activeResult,
                .completed,
                "terminal did not become active after fast reconnect on iteration \(iteration):\n\(uiStateSnapshot(app))",
                file: #filePath,
                line: #line
            )
        }

        assertTerminalCanType(app, marker: "BOO_UI_TYPED_STRESS")
    }

    func testKeyboardDismissAndRefocusStillTypes() {
        let app = makeApp(autoConnect: false, resetStorage: true)
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

    func testTerminalErrorBannerDoesNotOfferClientOwnedTabs() {
        let app = makeApp(
            autoConnect: false,
            resetStorage: true,
            forcedTerminalErrorKind: "authenticationFailed"
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        XCTAssertTrue(app.staticTexts["terminal-banner-label"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["new-tab-button"].exists)
        XCTAssertFalse(app.buttons["close-tab-button"].exists)
        XCTAssertTrue(app.buttons["disconnect-tab-button"].exists)
    }

    func testDisconnectErrorBannerReturnsToConnectScreen() {
        let app = makeApp(
            autoConnect: false,
            resetStorage: true,
            forcedTerminalErrorKind: "authenticationFailed"
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let disconnectButton = app.buttons["disconnect-tab-button"]
        XCTAssertTrue(disconnectButton.waitForExistence(timeout: 5))
        disconnectButton.tap()
        waitForConnectScreen(app)
    }

    func testConnectScreenElementsAppear() {
        let app = makeApp()
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)
        XCTAssertTrue(app.textFields["connect-host-input"].exists)
        XCTAssertTrue(app.buttons["connect-button"].exists)
    }

    func testOpenLiveTerminalCanDismissKeyboard() {
        let app = makeApp(autoConnect: false, resetStorage: true)
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
