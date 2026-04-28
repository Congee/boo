import XCTest

final class BooAppLaunchTests: BooUITestCase {
    private var isSimulatorRuntime: Bool {
        #if targetEnvironment(simulator)
        true
        #else
        false
        #endif
    }

    private struct RuntimePaneDebugFrame {
        let paneId: String
        let rect: CGRect
    }

    private func uiStateSnapshot(_ app: XCUIApplication) -> String {
        let title = app.staticTexts["screen-title"].exists ? app.staticTexts["screen-title"].label : "<none>"
        let connectScreen = app.otherElements["connect-screen"].exists
        let connectStatus = app.staticTexts["connect-status-banner"].exists ? app.staticTexts["connect-status-banner"].label : "<none>"
        let connectError = app.staticTexts["connect-error-label"].exists ? app.staticTexts["connect-error-label"].label : "<none>"
        let connectHostExists = app.textFields["connect-host-input"].exists
        let connectHostValue = connectHostExists ? String(describing: app.textFields["connect-host-input"].value ?? "<nil>") : "<none>"
        let terminalBanner = app.staticTexts["terminal-banner-label"].exists ? app.staticTexts["terminal-banner-label"].label : "<none>"
        let terminalDebug = app.otherElements["terminal-debug-state"].exists ? app.otherElements["terminal-debug-state"].label : "<none>"
        let terminalTrace = app.otherElements["terminal-trace-state"].exists ? app.otherElements["terminal-trace-state"].label : "<none>"
        let seedStatus = app.staticTexts["uitest-scroll-seed-status"].exists ? app.staticTexts["uitest-scroll-seed-status"].label : "<none>"
        let terminal = app.otherElements["terminal-screen"]
        let terminalExists = terminal.exists
        let terminalLabel = terminalExists ? terminal.label : "<none>"
        let terminalValue = terminalExists && !isSimulatorRuntime
            ? String(describing: terminal.value ?? "<nil>")
            : "<skipped>"
        let paneIdentifiers = runtimePaneElements(in: app)
            .prefix(6)
            .map(\.identifier)
            .joined(separator: ",")
        let floatingBack = app.buttons["floating-back-button"].exists
        return """
        title=\(title)
        connectScreen=\(connectScreen)
        connectStatus=\(connectStatus)
        connectError=\(connectError)
        connectHostExists=\(connectHostExists)
        connectHostValue=\(connectHostValue)
        terminalBanner=\(terminalBanner)
        terminalDebug=\(terminalDebug)
        terminalTrace=\(terminalTrace)
        seedStatus=\(seedStatus)
        terminalExists=\(terminalExists)
        terminalLabel=\(terminalLabel)
        terminalValuePrefix=\(terminalValue.prefix(200))
        paneIdentifiers=\(paneIdentifiers)
        floatingBack=\(floatingBack)
        """
    }

    private func runtimePaneElements(in app: XCUIApplication) -> [XCUIElement] {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH %@", "terminal-pane-"))
            .allElementsBoundByIndex
    }

    private func waitForRuntimePaneText(_ marker: String, in app: XCUIApplication, timeout: TimeInterval) -> Bool {
        let terminal = app.otherElements["terminal-screen"]
        guard terminal.waitForExistence(timeout: 2) else { return false }
        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value CONTAINS %@", marker),
            object: terminal
        )
        return XCTWaiter.wait(for: [expectation], timeout: timeout) == .completed
    }

    private func waitForUITestTraceOutput(in app: XCUIApplication, timeout: TimeInterval) -> Bool {
        let trace = app.otherElements["terminal-trace-state"]
        guard trace.waitForExistence(timeout: 2) else { return false }
        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(
                format: "label CONTAINS %@ AND label CONTAINS %@",
                "traceInputSent=true",
                "traceOutputObserved=true"
            ),
            object: trace
        )
        return XCTWaiter.wait(for: [expectation], timeout: timeout) == .completed
    }

    private func debugLabel(in app: XCUIApplication) -> String {
        let debug = app.otherElements["terminal-debug-state"]
        guard debug.exists else { return "" }
        return debug.label
    }

    private func focusedPaneId(in debugLabel: String) -> String? {
        guard let range = debugLabel.range(of: #"focusedPane=([0-9]+)"#, options: .regularExpression) else {
            return nil
        }
        let match = String(debugLabel[range])
        return match.split(separator: "=").last.map(String.init)
    }

    private func runtimePaneFrames(in debugLabel: String) -> [RuntimePaneDebugFrame] {
        guard let start = debugLabel.range(of: "paneFrames=[")?.upperBound,
              let end = debugLabel[start...].firstIndex(of: "]")
        else { return [] }
        let rawFrames = debugLabel[start..<end]
        return rawFrames.split(separator: ";").compactMap { entry in
            let parts = entry.split(separator: ":", maxSplits: 1)
            guard parts.count == 2 else { return nil }
            let coords = parts[1].split(separator: ",").compactMap { Double($0) }
            guard coords.count == 4 else { return nil }
            return RuntimePaneDebugFrame(
                paneId: String(parts[0]),
                rect: CGRect(
                    x: coords[0],
                    y: coords[1],
                    width: coords[2],
                    height: coords[3]
                )
            )
        }
    }

    private func tapRuntimePane(
        paneId: String,
        frames: [RuntimePaneDebugFrame],
        in app: XCUIApplication
    ) -> Bool {
        let paneElement = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == %@", "terminal-pane-\(paneId)"))
            .firstMatch
        if paneElement.waitForExistence(timeout: 2), paneElement.isHittable {
            paneElement.tap()
            return true
        }

        guard let frame = frames.first(where: { $0.paneId == paneId })?.rect else { return false }
        let terminal = app.otherElements["terminal-screen"]
        guard terminal.waitForExistence(timeout: 2) else { return false }

        let terminalFrame = terminal.frame
        let tapPoint = CGVector(
            dx: terminalFrame.minX + frame.midX,
            dy: terminalFrame.minY + frame.midY
        )
        app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            .withOffset(tapPoint)
            .tap()
        return true
    }

    private func waitForRuntimePaneFocus(
        paneId: String,
        in app: XCUIApplication,
        timeout: TimeInterval
    ) -> Bool {
        let debug = app.otherElements["terminal-debug-state"]
        guard debug.waitForExistence(timeout: 2) else { return false }
        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label CONTAINS %@", "focusedPane=\(paneId)"),
            object: debug
        )
        return XCTWaiter.wait(for: [expectation], timeout: timeout) == .completed
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
                savedNode.isHittable
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

    private func assertNoClientOwnedTerminalNavigationChrome(
        _ app: XCUIApplication,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertFalse(app.otherElements["terminal-connection-health-hud"].exists, file: file, line: line)
        XCTAssertFalse(app.buttons["Disconnect"].exists, file: file, line: line)
        XCTAssertFalse(app.buttons["Prev"].exists, file: file, line: line)
        XCTAssertFalse(app.buttons["Next"].exists, file: file, line: line)
        XCTAssertFalse(app.buttons["New"].exists, file: file, line: line)
        XCTAssertFalse(app.buttons["Close"].exists, file: file, line: line)
        XCTAssertFalse(app.staticTexts["Disconnect"].exists, file: file, line: line)
        XCTAssertFalse(app.staticTexts["Prev"].exists, file: file, line: line)
        XCTAssertFalse(app.staticTexts["Next"].exists, file: file, line: line)
        XCTAssertFalse(app.staticTexts["New"].exists, file: file, line: line)
        XCTAssertFalse(app.staticTexts["Close"].exists, file: file, line: line)
    }

    private func focusTerminalForTyping(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
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

        let proxy = app.textViews["terminal-text-proxy"]
        XCTAssertTrue(proxy.waitForExistence(timeout: 5), file: file, line: line)

        let keyboard = app.keyboards.firstMatch
        var keyboardVisible = keyboard.exists
        for _ in 0..<3 where !keyboardVisible {
            terminal.tap()
            keyboardVisible = keyboard.waitForExistence(timeout: 3)
        }
        XCTAssertTrue(keyboardVisible, "terminal keyboard never became visible: \(uiStateSnapshot(app))", file: file, line: line)
        return proxy
    }

    private func assertTerminalCanType(_ app: XCUIApplication, marker: String, file: StaticString = #filePath, line: UInt = #line) {
        let terminal = app.otherElements["terminal-screen"]
        let proxy = focusTerminalForTyping(app, file: file, line: line)
        let keyboard = app.keyboards.firstMatch

        var typedSuccessfully = false
        for _ in 0..<2 {
            if !keyboard.exists {
                terminal.tap()
                guard keyboard.waitForExistence(timeout: 3) else { continue }
            }
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
        XCTAssertTrue(terminal.label.hasPrefix("active-"), "terminal lost active tab after typing: \(activeTabStateSnapshot(app))", file: file, line: line)
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
            terminal.tap()
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

        let macMini = app.descendants(matching: .any).matching(identifier: "tailscale-peer-Mac mini").firstMatch
        let offlineBox = app.descendants(matching: .any).matching(identifier: "tailscale-peer-Offline box").firstMatch
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

    func testTerminalTopControlsAreHidden() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        XCTAssertFalse(app.buttons["floating-back-button"].exists)
        XCTAssertFalse(app.buttons["floating-disconnect-button"].exists)
        XCTAssertFalse(app.buttons["disconnect-tab-button"].exists)

        swipeBackFromTerminal(app)
        waitForConnectScreen(app)
        XCTAssertFalse(app.otherElements["terminal-screen"].exists)
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

    func testRuntimeViewThreePaneScreenshotAndTapFocus() {
        let marker = "BOO_IOS_RV_E2E"
        let app = makeApp(
            autoConnect: true,
            resetStorage: true,
            traceActions: "runtime-view-e2e,input",
            traceInputCommand: "printf '\(marker) 🙂 測試 é\\n'",
            traceOutputMarker: marker
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        if isConnectScreen(app) {
            connectToConfiguredBoo(from: app)
        }
        waitForTerminalScreen(app)

        let debug = app.otherElements["terminal-debug-state"]
        XCTAssertTrue(debug.waitForExistence(timeout: 10))

        let readyPredicate = NSPredicate(
            format: "label CONTAINS %@ AND label CONTAINS %@ AND label CONTAINS %@",
            "runtimeTabs=2",
            "visiblePanes=3",
            "paneStates=3"
        )
        let readyResult = XCTWaiter.wait(
            for: [XCTNSPredicateExpectation(predicate: readyPredicate, object: debug)],
            timeout: 20
        )
        XCTAssertEqual(readyResult, .completed, "runtime-view layout did not converge: \(uiStateSnapshot(app))")
        guard readyResult == .completed else { return }

        let panes = runtimePaneElements(in: app)
        XCTAssertGreaterThanOrEqual(panes.count, 3, "expected three accessibility-exposed runtime panes: \(uiStateSnapshot(app))")

        let outputObserved = isSimulatorRuntime
            ? waitForUITestTraceOutput(in: app, timeout: 30)
            : waitForRuntimePaneText(marker, in: app, timeout: 30)
        XCTAssertTrue(
            outputObserved,
            "runtime-view pane content stayed invisible or empty: \(uiStateSnapshot(app))"
        )
        guard outputObserved else { return }

        let afterContentAttachment = XCTAttachment(screenshot: app.screenshot())
        afterContentAttachment.name = "runtime-view-three-panes-after-content"
        afterContentAttachment.lifetime = .keepAlways
        add(afterContentAttachment)

        if isSimulatorRuntime {
            return
        }

        let currentDebug = debugLabel(in: app)
        let currentFocusedPaneId = focusedPaneId(in: currentDebug)
        let frames = runtimePaneFrames(in: currentDebug)
        XCTAssertGreaterThanOrEqual(frames.count, 3, "missing pane frame debug data: \(uiStateSnapshot(app))")
        guard frames.count >= 3 else { return }

        let target = frames
            .filter { $0.paneId != currentFocusedPaneId }
            .min { lhs, rhs in
                if lhs.rect.minY == rhs.rect.minY {
                    if lhs.rect.minX == rhs.rect.minX {
                        return lhs.paneId < rhs.paneId
                    }
                    return lhs.rect.minX < rhs.rect.minX
                }
                return lhs.rect.minY < rhs.rect.minY
            } ?? frames[0]
        XCTAssertTrue(
            tapRuntimePane(paneId: target.paneId, frames: frames, in: app),
            "could not tap runtime pane \(target.paneId): \(uiStateSnapshot(app))"
        )

        XCTAssertTrue(
            waitForRuntimePaneFocus(paneId: target.paneId, in: app, timeout: 12),
            "tapping pane \(target.paneId) did not move focus: \(uiStateSnapshot(app))"
        )

        let afterAttachment = XCTAttachment(screenshot: app.screenshot())
        afterAttachment.name = "runtime-view-three-panes-after-focus"
        afterAttachment.lifetime = .keepAlways
        add(afterAttachment)
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

        terminal.tap()
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
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        swipeBackFromTerminal(app)
        waitForConnectScreen(app)

        let metricBadge = app.staticTexts.matching(identifier: "host-metric-Local Boo").firstMatch
        XCTAssertTrue(metricBadge.waitForExistence(timeout: 8), "expected visible saved-host metric badge")

        let predicate = NSPredicate(format: "label MATCHES %@", "\\b[0-9]+ ms\\b")
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: metricBadge)
        let result = XCTWaiter.wait(for: [expectation], timeout: 8)
        XCTAssertEqual(
            result,
            .completed,
            "expected saved-host metric badge to contain latency text, got '\(metricBadge.label)'"
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

        let onlineAnyRow = app.descendants(matching: .any).matching(identifier: "tailscale-peer-Online Mac").firstMatch
        let offlineButtonRow = app.buttons["tailscale-peer-Offline box"]
        let offlineAnyRow = app.descendants(matching: .any).matching(identifier: "tailscale-peer-Offline box").firstMatch

        scrollUntilExists(onlineAnyRow, in: app)
        scrollUntilExists(offlineAnyRow, in: app)

        XCTAssertTrue(onlineAnyRow.exists)
        XCTAssertFalse(offlineButtonRow.exists, "offline Tailscale row should not be rendered as a tappable button")
        XCTAssertTrue(offlineAnyRow.exists, "offline Tailscale row should still be visible")

        let screenshotAttachment = XCTAttachment(screenshot: app.screenshot())
        screenshotAttachment.name = "offline-tailscale-row"
        screenshotAttachment.lifetime = .keepAlways
        add(screenshotAttachment)
    }

    func testUnreachableTailscalePortRowIsNotTappableAndDoesNotDuplicatePort() {
        let mockDevices = "Dead port|192.0.2.1|192.0.2.1|Linux|1"
        let app = makeApp(autoConnect: false, resetStorage: true, mockTailscaleDevices: mockDevices)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        navigateToConnectScreen(app)

        let row = app.descendants(matching: .any).matching(identifier: "tailscale-peer-Dead port").firstMatch
        scrollUntilExists(row, in: app)
        XCTAssertTrue(row.exists)

        let metricBadge = app.staticTexts.matching(identifier: "host-metric-Dead port").firstMatch
        XCTAssertTrue(metricBadge.waitForExistence(timeout: 10), "expected visible metric badge for unreachable port row")
        let unreachable = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label == %@", "unreachable"),
            object: metricBadge
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [unreachable], timeout: 10),
            .completed,
            "expected port probe to mark the row unreachable, got '\(metricBadge.label)'"
        )

        XCTAssertFalse(
            app.buttons["tailscale-peer-Dead port"].exists,
            "online Tailscale rows with an unreachable Boo port should not be rendered as tappable buttons"
        )
        let updatedRow = app.descendants(matching: .any).matching(identifier: "tailscale-peer-Dead port").firstMatch
        XCTAssertFalse(
            updatedRow.label.contains("boo:7337"),
            "row should show the probed port once as the colored port badge, not duplicate it in the subtitle: \(updatedRow.label)"
        )
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

        // Give live Tailscale/dashboard rows time to settle before capturing.
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

    func testSwipeBackReturnsToConnectScreen() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        swipeBackFromTerminal(app)

        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if isConnectScreen(app) {
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTFail("expected swipe-back to return to the connect screen")
    }

    func testReconnectAndTypeAgainAfterBackNavigation() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }
        assertTerminalCanType(app, marker: "BOO_UI_TYPED_1")

        swipeBackFromTerminal(app)
        waitForConnectScreen(app)

        guard openLiveTerminal(app) else { return }
        waitForTerminalScreen(app)
        assertTerminalCanType(app, marker: "BOO_UI_TYPED_2")
    }

    func testFastSwipeBackAndReconnectStress() {
        let app = makeApp(autoConnect: false, resetStorage: true)
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
            swipeBackFromTerminal(app)

            waitForConnectScreen(app)
            connectToConfiguredBoo(from: app)
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

        assertTerminalCanType(app, marker: "BOO_UI_TYPED_BEFORE_REFOCUS")

        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        let dismissButton = keyboardDismissButton(in: app)
        XCTAssertTrue(dismissButton.waitForExistence(timeout: 5))
        dismissButton.tap()
        XCTAssertFalse(keyboard.waitForExistence(timeout: 2))

        assertTerminalCanType(app, marker: "BOO_UI_TYPED_REFOCUS")
    }

    func testExitedTerminalReturnsToConnectScreenWithoutRecoveryBanner() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        let proxy = focusTerminalForTyping(app)
        proxy.typeText("exit\r")

        waitForConnectScreen(app)
        XCTAssertFalse(app.staticTexts["terminal-banner-label"].exists)
        XCTAssertFalse(app.buttons["recover-runtime-view-button"].exists)
    }

    func testOpeningTerminalTimeoutReturnsToConnectScreen() {
        let app = makeApp(
            autoConnect: false,
            resetStorage: true,
            includeConfiguredHost: false,
            forceOpeningTerminal: true,
            terminalOpeningTimeoutSeconds: 1
        )
        app.launchArguments.append("--boo-ui-test-host=opening-timeout.local")
        app.launchArguments.append("--boo-ui-test-port=7337")
        _ = installSystemAlertHandler(for: app)
        app.launch()

        waitForConnectScreen(app, timeout: 8)
        XCTAssertFalse(app.otherElements["terminal-opening-overlay"].exists)

        let error = app.staticTexts["connect-error-label"]
        XCTAssertTrue(error.exists)
        XCTAssertTrue(
            error.label.contains("Timed out opening a terminal tab"),
            "unexpected timeout error: \(error.label)"
        )
    }

    func testTerminalErrorDoesNotShowTerminalBannerOrClientOwnedTabs() {
        let app = makeApp(
            autoConnect: false,
            resetStorage: true,
            forcedTerminalErrorKind: "authenticationFailed"
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        XCTAssertFalse(app.staticTexts["terminal-banner-label"].exists)
        XCTAssertFalse(app.buttons["recover-runtime-view-button"].exists)
        XCTAssertFalse(app.buttons["reattach-runtime-view-button"].exists)
        XCTAssertFalse(app.buttons["new-tab-button"].exists)
        XCTAssertFalse(app.buttons["close-tab-button"].exists)
        XCTAssertFalse(app.buttons["disconnect-tab-button"].exists)
    }

    func testTerminalScreenHasNoClientOwnedNavigationChrome() {
        let app = makeApp(
            autoConnect: false,
            resetStorage: true,
            forceActiveTerminal: true
        )
        _ = installSystemAlertHandler(for: app)
        app.launch()
        waitForTerminalScreen(app)

        assertNoClientOwnedTerminalNavigationChrome(app)
    }

    func testLiveTerminalScreenHasNoClientOwnedNavigationChrome() {
        let app = makeApp(autoConnect: false, resetStorage: true)
        _ = installSystemAlertHandler(for: app)
        app.launch()
        app.tap()

        guard openLiveTerminal(app) else { return }

        assertNoClientOwnedTerminalNavigationChrome(app)
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

        XCTAssertFalse(app.buttons["disconnect-tab-button"].exists)
        swipeBackFromTerminal(app)
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
