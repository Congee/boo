import SwiftUI
import Network

struct BooRootView: View {
    @StateObject private var client = GSPClient()
    @StateObject private var browser = BonjourBrowser()
    @StateObject private var store = ConnectionStore()
    @State private var selectedTab: BooTab = .sessions
    @State private var monitor: ConnectionMonitor?

    private var activeMonitor: ConnectionMonitor {
        if let monitor { return monitor }
        let created = ConnectionMonitor(client: client)
        DispatchQueue.main.async { self.monitor = created }
        return created
    }

    var body: some View {
        ZStack(alignment: .bottom) {
            Group {
                if client.attachedSessionId != nil {
                    TerminalSessionScreen(client: client, monitor: activeMonitor, selectedTab: $selectedTab)
                } else {
                    switch selectedTab {
                    case .sessions:
                        if client.connected && client.authenticated {
                            SessionsScreen(client: client, monitor: activeMonitor, selectedTab: $selectedTab)
                        } else {
                            ConnectScreen(client: client, browser: browser, store: store, monitor: activeMonitor, selectedTab: $selectedTab)
                        }
                    case .history:
                        HistoryScreen(store: store)
                    case .settings:
                        SettingsScreen(store: store, monitor: activeMonitor)
                    }
                }
            }
            if client.attachedSessionId == nil {
                KineticTabBar(selectedTab: $selectedTab)
                    .padding(.bottom, KineticSpacing.md)
            }
        }
        .background(KineticColor.surface)
        .onAppear {
            if monitor == nil {
                monitor = ConnectionMonitor(client: client)
            }
        }
        .onChange(of: activeMonitor.status) { oldValue, newValue in
            handleStatusChange(from: oldValue, to: newValue)
        }
    }

    private func handleStatusChange(from oldValue: ConnectionStatus, to newValue: ConnectionStatus) {
        let wasConnected: Bool = {
            switch oldValue {
            case .connected, .authenticated, .attached:
                return true
            default:
                return false
            }
        }()
        switch newValue {
        case .authenticated, .attached:
            if let nodeId = activeMonitor.currentNodeId {
                store.updateNodeLastConnected(nodeId)
            }
        case .connectionLost:
            if let historyId = activeMonitor.currentHistoryId {
                store.endConnection(id: historyId, status: .timedOut)
                activeMonitor.clearTrackedConnection()
            }
        case .disconnected:
            if wasConnected, let historyId = activeMonitor.currentHistoryId {
                store.endConnection(id: historyId, status: .disconnected)
                activeMonitor.clearTrackedConnection()
            }
        default:
            break
        }
    }
}

struct ConnectScreen: View {
    @ObservedObject var client: GSPClient
    @ObservedObject var browser: BonjourBrowser
    @ObservedObject var store: ConnectionStore
    @ObservedObject var monitor: ConnectionMonitor
    @Binding var selectedTab: BooTab

    @State private var host = ""
    @State private var authKey = ""

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(
                title: "Connect to Server",
                subtitle: "Discover a Boo daemon on your local network or connect manually to a compatible remote endpoint."
            )
            ScrollView {
                VStack(alignment: .leading, spacing: KineticSpacing.xl) {
                    VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                        KineticSectionLabel(text: "Machine Address")
                        KineticInputField(placeholder: "hostname or ip:port", text: $host)
                        KineticSectionLabel(text: "Auth Key")
                        KineticInputField(placeholder: "optional shared secret", text: $authKey, secure: true)
                    }

                    if let error = client.lastError {
                        Text(error)
                            .font(KineticFont.caption)
                            .foregroundStyle(KineticColor.error)
                            .padding(KineticSpacing.md)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(KineticColor.error.opacity(0.1))
                            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                    }

                    VStack(spacing: KineticSpacing.sm) {
                        Button("Connect") { connectManual() }
                            .buttonStyle(KineticPrimaryButtonStyle())
                            .disabled(host.isEmpty)

                        Button("Settings") { selectedTab = .settings }
                            .buttonStyle(KineticSecondaryButtonStyle())
                    }

                    if !store.savedNodes.isEmpty {
                        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                            KineticSectionLabel(text: "Saved Nodes")
                            ForEach(store.savedNodes) { node in
                                KineticCardRow(
                                    icon: "server.rack",
                                    title: node.name,
                                    subtitle: "\(node.host):\(node.port)"
                                ) {
                                    let historyId = store.recordConnection(nodeName: node.name, host: node.host)
                                    monitor.connect(
                                        host: node.host,
                                        port: node.port,
                                        authKey: node.authKey,
                                        historyId: historyId,
                                        nodeId: node.id
                                    )
                                }
                            }
                        }
                    }

                    if !browser.daemons.isEmpty || browser.isSearching {
                        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                            KineticSectionLabel(text: "Discovered on Network")
                            if browser.isSearching && browser.daemons.isEmpty {
                                ProgressView()
                                    .tint(KineticColor.primary)
                            }
                            ForEach(browser.daemons) { daemon in
                                KineticCardRow(icon: "terminal", title: daemon.name, subtitle: "Bonjour service") {
                                    connectToEndpoint(daemon.endpoint)
                                }
                            }
                        }
                    }

                    if !store.history.isEmpty {
                        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                            KineticSectionLabel(text: "Recent Connections")
                            ForEach(store.history.prefix(5)) { entry in
                                KineticCardRow(icon: "clock.arrow.circlepath", title: entry.nodeName, subtitle: "\(entry.host) · \(entry.relativeTimeString)") {
                                    host = entry.host
                                }
                            }
                        }
                    }

                    Spacer().frame(height: 120)
                }
                .padding(.horizontal, KineticSpacing.md)
            }
        }
        .onAppear { browser.startBrowsing() }
        .onDisappear { browser.stopBrowsing() }
    }

    private func connectManual() {
        guard !host.isEmpty else { return }
        let parsed = parseHost(host)
        let historyId = store.recordConnection(nodeName: parsed.0, host: parsed.0)
        monitor.connect(host: parsed.0, port: parsed.1, authKey: authKey, historyId: historyId)
    }

    private func connectToEndpoint(_ endpoint: NWEndpoint) {
        switch endpoint {
        case .service(let name, _, _, _):
            let historyId = store.recordConnection(nodeName: name, host: name)
            monitor.connect(host: name, port: 7337, authKey: authKey, historyId: historyId)
        case .hostPort(let host, let port):
            let hostString = host.debugDescription
            let historyId = store.recordConnection(nodeName: hostString, host: hostString)
            monitor.connect(host: hostString, port: port.rawValue, authKey: authKey, historyId: historyId)
        default:
            break
        }
    }

    private func parseHost(_ raw: String) -> (String, UInt16) {
        if let index = raw.lastIndex(of: ":"), let port = UInt16(raw[raw.index(after: index)...]) {
            return (String(raw[..<index]), port)
        }
        return (raw, 7337)
    }
}

struct SessionsScreen: View {
    @ObservedObject var client: GSPClient
    @ObservedObject var monitor: ConnectionMonitor
    @Binding var selectedTab: BooTab

    private var subtitleText: String? {
        let base = monitor.lastHost.map { "Connected to \($0)" }
        guard let handshake = client.handshakeSummary else { return base }
        guard let base else { return handshake }
        return "\(base) · \(handshake)"
    }

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(
                title: "Active Sessions",
                subtitle: subtitleText
            )
            ScrollView {
                VStack(alignment: .leading, spacing: KineticSpacing.xl) {
                    if client.sessions.isEmpty {
                        Text("No active sessions")
                            .font(KineticFont.body)
                            .foregroundStyle(KineticColor.onSurfaceVariant)
                    } else {
                        ForEach(client.sessions) { session in
                            KineticCardRow(
                                icon: session.childExited ? "terminal" : "terminal.fill",
                                title: session.name.isEmpty ? (session.title.isEmpty ? "Session \(session.id)" : session.title) : session.name,
                                subtitle: session.pwd.isEmpty ? "PID \(session.id)" : session.pwd
                            ) {
                                client.attach(sessionId: session.id)
                            }
                        }
                    }

                    VStack(spacing: KineticSpacing.sm) {
                        Button("Create New Session") {
                            client.createSession()
                        }
                        .buttonStyle(KineticPrimaryButtonStyle())

                        Button("Disconnect") {
                            monitor.disconnect()
                        }
                        .buttonStyle(KineticSecondaryButtonStyle())
                    }

                    Spacer().frame(height: 120)
                }
                .padding(.horizontal, KineticSpacing.md)
            }
        }
        .onAppear { client.listSessions() }
    }
}

struct TerminalSessionScreen: View {
    @ObservedObject var client: GSPClient
    @ObservedObject var monitor: ConnectionMonitor
    @Binding var selectedTab: BooTab

    @State private var inputText = ""
    @State private var ctrlActive = false
    @State private var altActive = false

    private var sessionTitle: String {
        guard let sessionId = client.attachedSessionId else { return "Session" }
        if let session = client.sessions.first(where: { $0.id == sessionId }) {
            if !session.name.isEmpty { return session.name }
            if !session.title.isEmpty { return session.title }
        }
        return "Session \(sessionId)"
    }

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(
                title: sessionTitle,
                subtitle: statusSubtitle
            )

            if case .connectionLost(let reason) = monitor.status {
                Text(reason)
                    .font(KineticFont.caption)
                    .foregroundStyle(KineticColor.error)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(KineticSpacing.md)
                    .background(KineticColor.error.opacity(0.1))
            }

            RemoteTerminalView(screen: client.screen) { cols, rows in
                client.sendResize(cols: cols, rows: rows)
            }
            .opacity(isDisconnected ? 0.5 : 1.0)

            HStack(spacing: KineticSpacing.sm) {
                modifierButton("ESC") { client.sendInputBytes(Data([0x1b])) }
                modifierButton("CTRL", active: ctrlActive) { ctrlActive.toggle() }
                modifierButton("ALT", active: altActive) { altActive.toggle() }
                modifierButton("TAB") { client.sendInputBytes(Data([0x09])) }
                Spacer()
                modifierButton("↑") { client.sendInputBytes(Data([0x1b, 0x5b, 0x41])) }
                modifierButton("↓") { client.sendInputBytes(Data([0x1b, 0x5b, 0x42])) }
                modifierButton("←") { client.sendInputBytes(Data([0x1b, 0x5b, 0x44])) }
                modifierButton("→") { client.sendInputBytes(Data([0x1b, 0x5b, 0x43])) }
            }
            .padding(.horizontal, KineticSpacing.md)
            .padding(.vertical, KineticSpacing.sm)
            .background(KineticColor.surfaceContainerHigh.opacity(0.8))

            HStack(spacing: KineticSpacing.sm) {
                TextField("Type a command...", text: $inputText)
                    .font(KineticFont.monoInput)
                    .foregroundStyle(KineticColor.secondary)
                    .padding(KineticSpacing.md)
                    .background(KineticColor.surfaceContainerLowest)
                    .clipShape(RoundedRectangle(cornerRadius: KineticRadius.container))
                    .disabled(isDisconnected)
                    .onSubmit { sendCommand() }

                Button("Send") { sendCommand() }
                    .buttonStyle(KineticPrimaryButtonStyle())
                    .frame(width: 110)
            }
            .padding(KineticSpacing.md)
        }
        .background(KineticColor.surface)
    }

    private var isDisconnected: Bool {
        if case .connectionLost = monitor.status { return true }
        return false
    }

    private var statusSubtitle: String? {
        if case .connectionLost = monitor.status {
            return "Disconnected from daemon"
        }
        let base = monitor.lastHost.map { "Attached to \($0)" }
        guard let handshake = client.handshakeSummary else { return base }
        guard let base else { return handshake }
        return "\(base) · \(handshake)"
    }

    private func modifierButton(_ label: String, active: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(label)
                .font(KineticFont.caption)
                .fontWeight(.bold)
                .foregroundStyle(active ? KineticColor.surface : KineticColor.secondary)
                .padding(.horizontal, KineticSpacing.md)
                .padding(.vertical, KineticSpacing.sm)
                .background(active ? KineticColor.primary : KineticColor.surfaceContainerHighest)
                .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
        }
    }

    private func sendCommand() {
        guard !inputText.isEmpty else { return }
        var text = inputText
        if ctrlActive, let first = text.first, first.isLetter {
            let code = UInt8(first.uppercased().first!.asciiValue! - 64)
            client.sendInputBytes(Data([code]))
            ctrlActive = false
            inputText = ""
            return
        }
        if altActive {
            text = "\u{1b}" + text
            altActive = false
        }
        client.sendInput(text + "\r")
        inputText = ""
    }
}

struct HistoryScreen: View {
    @ObservedObject var store: ConnectionStore

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(title: "History", subtitle: "Recent connection activity.")
            ScrollView {
                VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                    if store.history.isEmpty {
                        Text("No connection history")
                            .font(KineticFont.body)
                            .foregroundStyle(KineticColor.onSurfaceVariant)
                    } else {
                        ForEach(store.history) { entry in
                            VStack(alignment: .leading, spacing: KineticSpacing.xs) {
                                Text(entry.nodeName)
                                    .font(KineticFont.bodySmall)
                                    .fontWeight(.bold)
                                    .foregroundStyle(KineticColor.onSurface)
                                Text("\(entry.host) · \(entry.relativeTimeString) · \(entry.durationString)")
                                    .font(KineticFont.caption)
                                    .foregroundStyle(KineticColor.onSurfaceVariant)
                            }
                            .padding(KineticSpacing.md)
                            .containerCard()
                        }
                    }
                    Spacer().frame(height: 120)
                }
                .padding(.horizontal, KineticSpacing.md)
            }
        }
    }
}

struct SettingsScreen: View {
    @ObservedObject var store: ConnectionStore
    @ObservedObject var monitor: ConnectionMonitor

    @State private var nodeName = ""
    @State private var nodeHost = ""
    @State private var nodePort = "7337"
    @State private var nodeAuthKey = ""

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(title: "Settings", subtitle: "Manage saved nodes and current connection state.")
            ScrollView {
                VStack(alignment: .leading, spacing: KineticSpacing.xl) {
                    VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                        KineticSectionLabel(text: "Save Node")
                        KineticInputField(placeholder: "Name", text: $nodeName)
                        KineticInputField(placeholder: "Host", text: $nodeHost)
                        KineticInputField(placeholder: "Port", text: $nodePort, keyboardType: .numberPad)
                        KineticInputField(placeholder: "Auth Key", text: $nodeAuthKey, secure: true)
                        Button("Save Node") { saveNode() }
                            .buttonStyle(KineticPrimaryButtonStyle())
                    }

                    if !store.savedNodes.isEmpty {
                        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                            KineticSectionLabel(text: "Saved Nodes")
                            ForEach(store.savedNodes) { node in
                                KineticCardRow(
                                    icon: "server.rack",
                                    title: node.name,
                                    subtitle: "\(node.host):\(node.port)"
                                )
                            }
                        }
                    }

                    Button("Disconnect") { monitor.disconnect() }
                        .buttonStyle(KineticSecondaryButtonStyle())

                    Button("Clear History") { store.clearHistory() }
                        .buttonStyle(KineticSecondaryButtonStyle())

                    Spacer().frame(height: 120)
                }
                .padding(.horizontal, KineticSpacing.md)
            }
        }
    }

    private func saveNode() {
        guard !nodeName.isEmpty, !nodeHost.isEmpty else { return }
        let port = UInt16(nodePort) ?? 7337
        store.addNode(SavedNode(name: nodeName, host: nodeHost, port: port, authKey: nodeAuthKey))
        nodeName = ""
        nodeHost = ""
        nodePort = "7337"
        nodeAuthKey = ""
    }
}
