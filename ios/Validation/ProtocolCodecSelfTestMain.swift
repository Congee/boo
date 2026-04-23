import Foundation

@main
struct ProtocolCodecSelfTestMain {
    static func main() {
        let tabs = WireCodec.decodeTabList(makeTabListPayload())
        assertEqual(tabs.count, 1, "tab list count")
        assertEqual(
            tabs[0],
            DecodedWireTabInfo(
                id: 7,
                name: "Tab 1",
                title: "shell",
                pwd: "/tmp",
                attached: true,
                childExited: false
            ),
            "tab list decoding"
        )
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601

        let runtimeStatePayload = """
        {
          "active_tab": 0,
          "focused_pane": 7,
          "tabs": [
            {
              "tab_id": 42,
              "index": 0,
              "active": true,
              "title": "shell",
              "pane_count": 1
            }
          ],
          "pwd": "/tmp"
        }
        """.data(using: .utf8)!
        let runtimeState = decodeRemoteRuntimeState(runtimeStatePayload)
        assertEqual(runtimeState?.activeTab, 0, "runtime state active tab decode")
        assertEqual(runtimeState?.tabs.first?.tabId, 42, "runtime state tab id decode")
        assertEqual(runtimeState?.focusedPane, 7, "runtime state focused pane decode")

        guard let state = WireCodec.decodeFullState(makeFullStatePayload()) else {
            fputs("failed to decode full-state payload\n", stderr)
            exit(1)
        }
        assertEqual(state.rows, 1, "rows decode")
        assertEqual(state.cols, 2, "cols decode")
        assertEqual(state.cursorX, 1, "cursorX decode")
        assertEqual(state.cursorY, 0, "cursorY decode")
        assertEqual(state.cursorVisible, true, "cursor visible decode")
        assertEqual(state.cells.count, 2, "cell count decode")
        assertEqual(WireCodec.screenText(from: state), "A好", "screen text decoding")

        var deltaState = state
        let applied = WireCodec.applyDelta(makeDeltaPayload(), to: &deltaState)
        assertEqual(applied, true, "delta apply succeeds")
        assertEqual(deltaState.cursorX, 0, "delta cursorX decode")
        assertEqual(deltaState.cursorY, 0, "delta cursorY decode")
        assertEqual(deltaState.cursorVisible, true, "delta cursor visible decode")
        assertEqual(WireCodec.screenText(from: deltaState), "BC", "delta screen text decoding")

        var clientState = ClientWireState()
        let buildId = "0.1.0"
        let serverInstanceId = "deadbeefcafebabe"
        let serverIdentityId = "daemon-identity-01"
        var authOkPayload = Data(count: 12 + buildId.utf8.count + serverInstanceId.utf8.count + serverIdentityId.utf8.count)
        authOkPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(1).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt32(0x7f).littleEndian, toByteOffset: 2, as: UInt32.self)
            bytes.storeBytes(of: UInt16(buildId.utf8.count).littleEndian, toByteOffset: 6, as: UInt16.self)
        }
        authOkPayload.replaceSubrange(8..<(8 + buildId.utf8.count), with: buildId.utf8)
        let instanceLengthOffset = 8 + buildId.utf8.count
        authOkPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(serverInstanceId.utf8.count).littleEndian, toByteOffset: instanceLengthOffset, as: UInt16.self)
        }
        authOkPayload.replaceSubrange(
            (instanceLengthOffset + 2)..<(instanceLengthOffset + 2 + serverInstanceId.utf8.count),
            with: serverInstanceId.utf8
        )
        let identityLengthOffset = instanceLengthOffset + 2 + serverInstanceId.utf8.count
        authOkPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(serverIdentityId.utf8.count).littleEndian, toByteOffset: identityLengthOffset, as: UInt16.self)
        }
        authOkPayload.replaceSubrange(
            (identityLengthOffset + 2)..<(identityLengthOffset + 2 + serverIdentityId.utf8.count),
            with: serverIdentityId.utf8
        )
        let authEffect = ClientWireReducer.reduce(message: .authOk, payload: authOkPayload, state: &clientState)
        assertEqual(authEffect, .listTabs, "auth ok triggers tab refresh")
        assertEqual(clientState.authenticated, true, "auth ok sets authenticated")
        assertEqual(clientState.protocolVersion, 1, "auth ok protocol version decode")
        assertEqual(clientState.transportCapabilities, 0x7f, "auth ok capability decode")
        assertEqual(clientState.serverBuildId, buildId, "auth ok build id decode")
        assertEqual(clientState.serverInstanceId, serverInstanceId, "auth ok instance id decode")
        assertEqual(clientState.serverIdentityId, serverIdentityId, "auth ok identity id decode")
        assertEqual(validateAuthOkMetadata(authOkPayload), nil, "auth ok metadata validation")

        var missingBuildPayload = Data(count: 6)
        missingBuildPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(1).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt32(0x7f).littleEndian, toByteOffset: 2, as: UInt32.self)
        }
        assertEqual(
            validateAuthOkMetadata(missingBuildPayload),
            "Remote handshake is missing server build metadata",
            "missing build metadata rejected"
        )

        var wrongVersionPayload = authOkPayload
        wrongVersionPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(2).littleEndian, as: UInt16.self)
        }
        assertEqual(
            validateAuthOkMetadata(wrongVersionPayload),
            "Unsupported remote protocol version: 2",
            "wrong protocol version rejected"
        )

        var missingHeartbeatPayload = authOkPayload
        missingHeartbeatPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt32(0x0f).littleEndian, toByteOffset: 2, as: UInt32.self)
        }
        assertEqual(
            validateAuthOkMetadata(missingHeartbeatPayload),
            "Remote server does not advertise heartbeat support",
            "missing heartbeat capability rejected"
        )

        var missingIdentityCapabilityPayload = authOkPayload
        missingIdentityCapabilityPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt32(0x5f).littleEndian, toByteOffset: 2, as: UInt32.self)
        }
        assertEqual(
            validateAuthOkMetadata(missingIdentityCapabilityPayload),
            "Remote server does not advertise daemon identity support",
            "missing daemon identity capability rejected"
        )
        assertEqual(
            serverIdentityMismatch(expectedIdentityId: "daemon-a", actualIdentityId: "daemon-b"),
            true,
            "server identity mismatch detects changed daemon"
        )
        assertEqual(
            serverIdentityMismatch(expectedIdentityId: "daemon-a", actualIdentityId: "daemon-a"),
            false,
            "server identity mismatch accepts same daemon"
        )
        assertEqual(
            serverIdentityMismatch(expectedIdentityId: nil, actualIdentityId: "daemon-a"),
            false,
            "server identity mismatch ignores unknown trusted identity"
        )

        let listEffect = ClientWireReducer.reduce(message: .tabList, payload: makeTabListPayload(), state: &clientState)
        assertEqual(listEffect, .none, "tab list has no side effect")
        assertEqual(clientState.tabs, tabs, "tab list reducer decode")

        let createdPayload = UInt32(42).littleEndianBytes
        let createdEffect = ClientWireReducer.reduce(message: .tabCreated, payload: Data(createdPayload), state: &clientState)
        assertEqual(createdEffect, .none, "tab created no longer triggers client-side attach")

        clientState.screen = state
        let deltaEffect = ClientWireReducer.reduce(message: .delta, payload: makeDeltaPayload(), state: &clientState)
        assertEqual(deltaEffect, .none, "delta has no side effect")
        assertEqual(clientState.screen.map(WireCodec.screenText(from:)), "BC", "delta reducer applies screen update")

        clientState.attachedTabId = 42
        let tabExitedEffect = ClientWireReducer.reduce(message: .tabExited, payload: Data(), state: &clientState)
        assertEqual(tabExitedEffect, .none, "tab exited has no side effect")
        assertEqual(clientState.attachedTabId, nil, "tab exited clears attached tab")

        let reachableTabs = [
            RemoteTabInfo(
                id: 42,
                name: "Tab 1",
                title: "shell",
                pwd: "/tmp",
                attached: true,
                childExited: false
            )
        ]
        assertEqual(
            resolveAttachedTabHealth(attachedTabId: nil, tabs: reachableTabs),
            .unattached,
            "missing attachment is unhealthy"
        )
        assertEqual(
            resolveAttachedTabHealth(attachedTabId: 7, tabs: reachableTabs),
            .unreachable(tabId: 7),
            "missing attached tab is unreachable"
        )
        assertEqual(
            resolveAttachedTabHealth(attachedTabId: 42, tabs: reachableTabs),
            .reachable(tabId: 42),
            "live attached tab is reachable"
        )
        assertEqual(
            resolveAttachedTabHealth(
                attachedTabId: 9,
                tabs: [
                    RemoteTabInfo(
                        id: 9,
                        name: "Tab 9",
                        title: "shell",
                        pwd: "/tmp",
                        attached: true,
                        childExited: true
                    )
                ]
            ),
            .exited(tabId: 9),
            "exited tab is not treated as reachable"
        )

        print("iOS wire codec self-test passed")
    }
}

private func tryOrExit<T>(_ result: @autoclosure () throws -> T, _ context: String) -> T {
    do {
        return try result()
    } catch {
        fputs("\(context): \(error)\n", stderr)
        exit(1)
    }
}
