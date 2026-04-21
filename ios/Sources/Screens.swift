import SwiftUI
import Network

private func formatConnectionTarget(host: String, port: UInt16) -> String {
    port == 7337 ? host : "\(host):\(port)"
}

private func endpointDisplayTarget(_ endpoint: NWEndpoint) -> (nodeName: String, host: String, port: UInt16) {
    switch endpoint {
    case .service(let name, _, _, _):
        return (name, name, 7337)
    case .hostPort(let host, let port):
        let hostString = host.debugDescription
        return (hostString, hostString, port.rawValue)
    default:
        let text = "\(endpoint)"
        return (text, text, 7337)
    }
}

struct BooRootView: View {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var client = GSPClient()
    @StateObject private var browser = BonjourBrowser()
    @StateObject private var tailscaleBrowser = TailscalePeerBrowser()
    @StateObject private var store = ConnectionStore()
    @State private var selectedTab: BooTab = .sessions
    @State private var monitor: ConnectionMonitor?
    @State private var serverIdentityWarning: String?
    @State private var didApplyUITestLaunchConfiguration = false

    private var activeMonitor: ConnectionMonitor {
        if let monitor { return monitor }
        let created = ConnectionMonitor(client: client, store: store)
        DispatchQueue.main.async { self.monitor = created }
        return created
    }

    var body: some View {
        ZStack(alignment: .bottom) {
            Group {
                if client.attachedSessionId != nil {
                    TerminalSessionScreen(client: client, monitor: activeMonitor, selectedTab: $selectedTab, serverIdentityWarning: serverIdentityWarning)
                } else {
                    switch selectedTab {
                    case .sessions:
                        if client.connected && client.authenticated {
                            SessionsScreen(client: client, monitor: activeMonitor, selectedTab: $selectedTab, serverIdentityWarning: serverIdentityWarning)
                        } else {
                            ConnectScreen(
                                client: client,
                                browser: browser,
                                tailscaleBrowser: tailscaleBrowser,
                                store: store,
                                monitor: activeMonitor,
                                selectedTab: $selectedTab,
                                serverIdentityWarning: serverIdentityWarning
                            )
                        }
                    case .history:
                        HistoryScreen(store: store)
                    case .settings:
                        SettingsScreen(
                            client: client,
                            store: store,
                            tailscaleBrowser: tailscaleBrowser,
                            monitor: activeMonitor,
                            serverIdentityWarning: $serverIdentityWarning
                        )
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
                monitor = ConnectionMonitor(client: client, store: store)
            }
            applyUITestLaunchConfigurationIfNeeded()
        }
        .onChange(of: activeMonitor.status) { oldValue, newValue in
            handleStatusChange(from: oldValue, to: newValue)
        }
        .onChange(of: scenePhase) { _, newPhase in
            guard newPhase == .active else { return }
            guard activeMonitor.lastHost != nil, !client.connected else { return }
            activeMonitor.reconnect()
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
            if let host = activeMonitor.lastHost,
               let port = activeMonitor.lastPort,
               let serverIdentityId = client.serverIdentityId,
               !serverIdentityId.isEmpty
            {
                serverIdentityWarning = store.recordTrustedServerIdentity(
                    host: host,
                    port: port,
                    identityId: serverIdentityId
                )
                if let sessionId = client.attachedSessionId,
                   let attachmentId = client.attachmentId,
                   let resumeToken = client.resumeToken
                {
                    store.recordResumeAttachment(
                        host: host,
                        port: port,
                        sessionId: sessionId,
                        attachmentId: attachmentId,
                        resumeToken: resumeToken
                    )
                }
            }
        case .connectionLost:
            if let historyId = activeMonitor.currentHistoryId {
                store.endConnection(id: historyId, status: .timedOut)
                activeMonitor.clearTrackedConnection()
            }
        case .disconnected:
            if let host = activeMonitor.lastHost, let port = activeMonitor.lastPort {
                store.clearResumeAttachment(host: host, port: port)
            }
            if wasConnected, let historyId = activeMonitor.currentHistoryId {
                store.endConnection(id: historyId, status: .disconnected)
                activeMonitor.clearTrackedConnection()
            }
        default:
            break
        }
    }

    private func applyUITestLaunchConfigurationIfNeeded() {
        guard !didApplyUITestLaunchConfiguration else { return }
        didApplyUITestLaunchConfiguration = true
        guard let config = UITestLaunchConfiguration.current(),
              config.autoConnect,
              let host = config.host
        else { return }

        let matchingNodeId = store.savedNodes.first {
            $0.host == host && $0.port == config.port && $0.authKey == config.authKey
        }?.id
        let historyId = store.recordConnection(
            nodeName: config.nodeName ?? host,
            host: formatConnectionTarget(host: host, port: config.port)
        )
        activeMonitor.connect(
            host: host,
            port: config.port,
            authKey: config.authKey,
            historyId: historyId,
            nodeId: matchingNodeId
        )
    }
}

struct ConnectScreen: View {
    @ObservedObject var client: GSPClient
    @ObservedObject var browser: BonjourBrowser
    @ObservedObject var tailscaleBrowser: TailscalePeerBrowser
    @ObservedObject var store: ConnectionStore
    @ObservedObject var monitor: ConnectionMonitor
    @Binding var selectedTab: BooTab
    let serverIdentityWarning: String?

    @State private var host = ""
    @State private var authKey = ""

    private var statusBanner: (message: String, color: Color)? {
        switch monitor.reconnectState {
        case .waiting(let attempt, _):
            return ("Reconnecting to saved host (attempt \(attempt))", KineticColor.primary)
        case .failed(let reason):
            return ("Reconnect failed: \(reason)", KineticColor.error)
        case .idle:
            break
        }
        switch monitor.transportHealth {
        case .degraded(let reason):
            return (reason, KineticColor.tertiary)
        case .lost(let reason):
            return (reason, KineticColor.error)
        default:
            break
        }
        switch monitor.status {
        case .connecting:
            return ("Connecting…", KineticColor.primary)
        case .connectionLost(let reason):
            return (reason, KineticColor.error)
        default:
            return nil
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(
                title: "Connect to Server",
                subtitle: "Discover a Boo daemon on your local network or connect manually to a compatible remote endpoint."
            )
            ScrollView {
                VStack(alignment: .leading, spacing: KineticSpacing.xl) {
                    if let statusBanner {
                        Text(statusBanner.message)
                            .font(KineticFont.caption)
                            .foregroundStyle(statusBanner.color)
                            .padding(KineticSpacing.md)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(statusBanner.color.opacity(0.1))
                            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                    }
                    if let serverIdentityWarning {
                        Text(serverIdentityWarning)
                            .font(KineticFont.caption)
                            .foregroundStyle(KineticColor.error)
                            .padding(KineticSpacing.md)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(KineticColor.error.opacity(0.1))
                            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                    }

                    VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                        KineticSectionLabel(text: "Machine Address")
                        KineticInputField(placeholder: "hostname or ip:port", text: $host, accessibilityIdentifier: "connect-host-input")
                        KineticSectionLabel(text: "Auth Key")
                        KineticInputField(placeholder: "optional shared secret", text: $authKey, secure: true, accessibilityIdentifier: "connect-authkey-input")
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
                            .accessibilityIdentifier("connect-button")

                        Button("Settings") { selectedTab = .settings }
                            .buttonStyle(KineticSecondaryButtonStyle())
                            .accessibilityIdentifier("settings-button")
                    }

                    if !store.savedNodes.isEmpty {
                        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                            KineticSectionLabel(text: "Saved Nodes")
                            ForEach(store.savedNodes) { node in
                                KineticCardRow(
                                    icon: "server.rack",
                                    title: node.name,
                                    subtitle: "\(node.host):\(node.port)",
                                    onTap: {
                                        let historyId = store.recordConnection(
                                            nodeName: node.name,
                                            host: formatConnectionTarget(host: node.host, port: node.port)
                                        )
                                        monitor.connect(
                                            host: node.host,
                                            port: node.port,
                                            authKey: node.authKey,
                                            historyId: historyId,
                                            nodeId: node.id
                                        )
                                    },
                                    accessibilityIdentifier: "saved-node-\(node.name)"
                                )
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
                                KineticCardRow(
                                    icon: "terminal",
                                    title: daemon.name,
                                    subtitle: "Bonjour service",
                                    onTap: {
                                        connectToEndpoint(daemon.endpoint)
                                    },
                                    accessibilityIdentifier: "discovered-daemon-\(daemon.name)"
                                )
                            }
                        }
                    }

                    if store.hasTailscaleAPIToken || tailscaleBrowser.isLoading || !tailscaleBrowser.peers.isEmpty || tailscaleBrowser.lastError != nil {
                        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                            KineticSectionLabel(text: "Tailscale Devices")
                            Text("Tailnet devices from the Tailscale API. Boo still needs to be running on the configured port.")
                                .font(KineticFont.caption)
                                .foregroundStyle(KineticColor.onSurfaceVariant)
                            if tailscaleBrowser.isLoading {
                                ProgressView()
                                    .tint(KineticColor.primary)
                            }
                            if let error = tailscaleBrowser.lastError {
                                Text(error)
                                    .font(KineticFont.caption)
                                    .foregroundStyle(KineticColor.error)
                                    .padding(KineticSpacing.md)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .background(KineticColor.error.opacity(0.1))
                                    .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                            }
                            ForEach(tailscaleBrowser.peers) { peer in
                                let state = peer.online ? "online" : "offline"
                                let detail = [peer.os, state, peer.address, "boo:\(peer.port)"].compactMap { $0 }.joined(separator: " · ")
                                KineticCardRow(
                                    icon: "network",
                                    title: peer.name,
                                    subtitle: detail,
                                    onTap: {
                                        connectToHost(peer.host, port: peer.port, nodeName: peer.name)
                                    },
                                    accessibilityIdentifier: "tailscale-peer-\(peer.name)"
                                )
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
        .onAppear {
            browser.startBrowsing()
            tailscaleBrowser.refresh(store: store)
        }
        .onDisappear {
            browser.stopBrowsing()
            tailscaleBrowser.stop()
        }
        .onChange(of: store.tailscaleDiscoverySettings) { _, _ in
            tailscaleBrowser.refresh(store: store)
        }
    }

    private func connectManual() {
        guard !host.isEmpty else { return }
        let parsed = parseHost(host)
        connectToHost(parsed.0, port: parsed.1, nodeName: parsed.0)
    }

    private func connectToEndpoint(_ endpoint: NWEndpoint) {
        let display = endpointDisplayTarget(endpoint)
        let historyId = store.recordConnection(
            nodeName: display.nodeName,
            host: formatConnectionTarget(host: display.host, port: display.port)
        )
        monitor.connect(
            endpoint: endpoint,
            displayHost: display.host,
            displayPort: display.port,
            authKey: authKey,
            historyId: historyId
        )
    }

    private func connectToHost(_ host: String, port: UInt16, nodeName: String) {
        let historyId = store.recordConnection(
            nodeName: nodeName,
            host: formatConnectionTarget(host: host, port: port)
        )
        monitor.connect(host: host, port: port, authKey: authKey, historyId: historyId)
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
    let serverIdentityWarning: String?

    private var activeSessions: [SessionInfo] {
        client.sessions.filter { !$0.childExited }
    }

    private var hasLiveSessionList: Bool {
        guard client.connected, client.authenticated else { return false }
        if case .lost = monitor.transportHealth {
            return false
        }
        return true
    }

    private var visibleSessions: [SessionInfo] {
        hasLiveSessionList ? activeSessions : []
    }

    private var emptyStateTitle: String {
        hasLiveSessionList ? "No open sessions" : "Session list unavailable"
    }

    private var emptyStateSubtitle: String {
        hasLiveSessionList
            ? "Create a new session or reconnect to refresh the list."
            : "Reconnect to the server to refresh reachable sessions."
    }

    private var connectionSummary: String? {
        switch monitor.status {
        case .attached:
            return monitor.lastHost.map { "Connected to \($0)" } ?? "Connected"
        case .authenticated, .connected:
            return monitor.lastHost.map { "Connected to \($0)" } ?? "Connected"
        case .connecting:
            return monitor.lastHost.map { "Connecting to \($0)" } ?? "Connecting"
        case .connectionLost:
            return monitor.lastHost.map { "Connection lost to \($0)" } ?? "Connection lost"
        case .disconnected:
            return monitor.lastHost.map { "Disconnected from \($0)" } ?? "Disconnected"
        }
    }

    private var connectionBannerText: String? {
        if let serverIdentityWarning {
            return serverIdentityWarning
        }
        switch monitor.transportHealth {
        case .degraded(let reason):
            return reason
        case .lost(let reason):
            return reason
        case .idle, .healthy:
            break
        }
        switch monitor.reconnectState {
        case .waiting(let attempt, _):
            return "Reconnecting (attempt \(attempt))"
        case .failed(let reason):
            return "Reconnect failed: \(reason)"
        case .idle:
            return nil
        }
    }

    private var connectionBannerColor: Color {
        if serverIdentityWarning != nil {
            return KineticColor.error
        }
        switch monitor.transportHealth {
        case .degraded:
            return KineticColor.tertiary
        case .lost:
            return KineticColor.error
        case .idle, .healthy:
            break
        }
        switch monitor.reconnectState {
        case .waiting:
            return KineticColor.primary
        case .failed:
            return KineticColor.error
        case .idle:
            return KineticColor.tertiary
        }
    }

    private func sessionTitle(_ session: SessionInfo) -> String {
        if !session.name.isEmpty { return session.name }
        if !session.title.isEmpty { return session.title }
        return "Session \(session.id)"
    }

    private func sessionSubtitle(_ session: SessionInfo) -> String {
        if !session.pwd.isEmpty { return session.pwd }
        if !session.title.isEmpty { return session.title }
        return "Ready"
    }

    private func sessionStatus(_ session: SessionInfo) -> String {
        if client.pendingAttachedSessionId == session.id {
            return "Opening"
        }
        if client.attachedSessionId == session.id {
            return "Current"
        }
        if session.attached {
            return "In Use"
        }
        return "Open"
    }

    private func sessionStatusColor(_ session: SessionInfo) -> Color {
        if client.pendingAttachedSessionId == session.id {
            return KineticColor.primary
        }
        if client.attachedSessionId == session.id {
            return KineticColor.primary
        }
        if session.attached {
            return KineticColor.tertiary
        }
        return KineticColor.onSurfaceVariant
    }

    @ViewBuilder
    private func sessionRow(_ session: SessionInfo) -> some View {
        Button {
            client.attach(sessionId: session.id)
        } label: {
            HStack(spacing: KineticSpacing.md) {
                Image(systemName: client.attachedSessionId == session.id ? "terminal.fill" : "terminal")
                    .font(.system(size: 20))
                    .foregroundStyle(KineticColor.primary)
                    .frame(width: 40, height: 40)
                    .background(KineticColor.surfaceContainerHighest)
                    .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                VStack(alignment: .leading, spacing: KineticSpacing.xs) {
                    Text(sessionTitle(session))
                        .font(KineticFont.bodySmall)
                        .fontWeight(.bold)
                        .foregroundStyle(KineticColor.onSurface)
                    Text(sessionSubtitle(session))
                        .font(KineticFont.caption)
                        .foregroundStyle(KineticColor.onSurfaceVariant)
                }
                Spacer()
                Text(sessionStatus(session).uppercased())
                    .font(KineticFont.sectionLabel)
                    .tracking(1)
                    .foregroundStyle(sessionStatusColor(session))
            }
            .padding(KineticSpacing.md)
            .containerCard()
        }
        .buttonStyle(.plain)
        .disabled(client.pendingAttachedSessionId != nil)
        .accessibilityIdentifier("session-row-\(session.id)")
    }

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(
                title: "Active Sessions",
                subtitle: connectionSummary
            )
            if let connectionBannerText {
                transportBanner(reason: connectionBannerText, color: connectionBannerColor)
            }
            ScrollView {
                VStack(alignment: .leading, spacing: KineticSpacing.xl) {
                    if hasLiveSessionList, !visibleSessions.isEmpty {
                        KineticSectionLabel(text: "Open Tabs")
                    }
                    if visibleSessions.isEmpty {
                        VStack(alignment: .leading, spacing: KineticSpacing.xs) {
                            Text(emptyStateTitle)
                                .font(KineticFont.body)
                                .fontWeight(.bold)
                                .foregroundStyle(KineticColor.onSurface)
                            Text(emptyStateSubtitle)
                                .font(KineticFont.caption)
                                .foregroundStyle(KineticColor.onSurfaceVariant)
                        }
                        .padding(KineticSpacing.md)
                        .containerCard()
                    } else {
                        ForEach(visibleSessions) { session in
                            sessionRow(session)
                        }
                    }

                    VStack(spacing: KineticSpacing.sm) {
                        Button("Create New Session") {
                            client.createSession()
                        }
                        .buttonStyle(KineticPrimaryButtonStyle())
                        .disabled(!hasLiveSessionList)
                        .accessibilityIdentifier("create-session-button")

                        Button("Disconnect") {
                            monitor.disconnect()
                        }
                        .buttonStyle(KineticSecondaryButtonStyle())
                        .accessibilityIdentifier("sessions-disconnect-button")
                    }

                    Spacer().frame(height: 120)
                }
                .padding(.horizontal, KineticSpacing.md)
            }
        }
        .onAppear {
            if client.authenticated {
                client.listSessions()
            }
        }
    }

    private var transportSummary: String? {
        switch monitor.transportHealth {
        case .idle:
            return nil
        case .healthy:
            return "transport healthy"
        case .degraded(let reason):
            return "transport degraded: \(reason)"
        case .lost(let reason):
            return "transport lost: \(reason)"
        }
    }

    private var reconnectSummary: String? {
        switch monitor.reconnectState {
        case .idle:
            return nil
        case .waiting(let attempt, _):
            return "reconnecting (\(attempt))"
        case .failed(let reason):
            return "reconnect failed: \(reason)"
        }
    }

    private func transportBanner(reason: String, color: Color) -> some View {
        Text(reason)
            .font(KineticFont.caption)
            .foregroundStyle(color)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(KineticSpacing.md)
            .background(color.opacity(0.1))
    }
}

struct TerminalSessionScreen: View {
    @ObservedObject var client: GSPClient
    @ObservedObject var monitor: ConnectionMonitor
    @Binding var selectedTab: BooTab
    let serverIdentityWarning: String?

    @State private var keyboardFocused = false
    @State private var ctrlActive = false
    @State private var altActive = false
    @State private var metaActive = false

    private var sessionHealth: AttachedSessionHealth {
        resolveAttachedSessionHealth(attachedSessionId: client.attachedSessionId, sessions: client.sessions)
    }

    private var attachedSession: SessionInfo? {
        guard let sessionId = client.attachedSessionId else { return nil }
        return client.sessions.first(where: { $0.id == sessionId })
    }

    private var sessionHealthIssue: String? {
        sessionHealth.issue
    }

    private var sessionTitle: String {
        guard let sessionId = client.attachedSessionId else { return "Session" }
        if let session = attachedSession {
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

            if let serverIdentityWarning {
                transportBanner(reason: serverIdentityWarning, color: KineticColor.error)
            } else if let sessionHealthIssue {
                transportBanner(reason: sessionHealthIssue, color: KineticColor.error)
            } else if let lastError = client.lastError, !lastError.isEmpty {
                transportBanner(reason: lastError, color: KineticColor.error)
            } else if let disconnectReason {
                transportBanner(reason: disconnectReason, color: KineticColor.error)
            } else if case .degraded(let reason) = monitor.transportHealth {
                transportBanner(reason: reason, color: KineticColor.tertiary)
            }

            RemoteTerminalView(screen: client.screen) { cols, rows in
                client.sendResize(cols: cols, rows: rows)
            } onGestureAction: { action in
                handleTerminalGesture(action)
            }
            .opacity(isDisconnected ? 0.5 : 1.0)
            .accessibilityIdentifier("terminal-screen")
            .accessibilityValue(client.screen.accessibilityTextSnapshot)
            .contentShape(Rectangle())
            .onTapGesture {
                guard !isDisconnected else { return }
                keyboardFocused = true
            }
            .overlay {
                TerminalKeyboardBridge(isFocused: $keyboardFocused) { text in
                    sendTypedText(text)
                } onBackspace: {
                    client.sendInputBytes(Data([0x7f]))
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .contentShape(Rectangle())
            }

            HStack {
                Button {
                    client.detach()
                    selectedTab = .sessions
                } label: {
                    Label("Sessions", systemImage: "sidebar.left")
                        .font(KineticFont.caption)
                        .fontWeight(.bold)
                        .foregroundStyle(KineticColor.secondary)
                        .padding(.horizontal, KineticSpacing.md)
                        .padding(.vertical, KineticSpacing.sm)
                        .background(KineticColor.surfaceContainerHighest)
                        .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                }
                Spacer()
            }
            .padding(.horizontal, KineticSpacing.md)
            .padding(.top, KineticSpacing.sm)
            .padding(.bottom, KineticSpacing.xs)
            .background(KineticColor.surfaceContainerHigh.opacity(0.45))

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: KineticSpacing.sm) {
                    modifierButton("ESC") { sendSpecialKey([0x1b]) }
                    modifierButton("CTRL", active: ctrlActive) { ctrlActive.toggle() }
                    modifierButton("ALT", active: altActive) { altActive.toggle() }
                    modifierButton("META", active: metaActive) { metaActive.toggle() }
                    modifierButton("TAB") { sendSpecialKey([0x09]) }
                    modifierButton("HOME") { sendSpecialKey([0x1b, 0x5b, 0x48]) }
                    modifierButton("END") { sendSpecialKey([0x1b, 0x5b, 0x46]) }
                    modifierButton("PG↑") { sendSpecialKey([0x1b, 0x5b, 0x35, 0x7e]) }
                    modifierButton("PG↓") { sendSpecialKey([0x1b, 0x5b, 0x36, 0x7e]) }
                    modifierButton("F1") { sendSpecialKey([0x1b, 0x4f, 0x50]) }
                    modifierButton("F2") { sendSpecialKey([0x1b, 0x4f, 0x51]) }
                    modifierButton("F3") { sendSpecialKey([0x1b, 0x4f, 0x52]) }
                    modifierButton("F4") { sendSpecialKey([0x1b, 0x4f, 0x53]) }
                    modifierButton("↑") { sendSpecialKey([0x1b, 0x5b, 0x41]) }
                    modifierButton("↓") { sendSpecialKey([0x1b, 0x5b, 0x42]) }
                    modifierButton("←") { sendSpecialKey([0x1b, 0x5b, 0x44]) }
                    modifierButton("→") { sendSpecialKey([0x1b, 0x5b, 0x43]) }
                }
                .padding(.horizontal, KineticSpacing.md)
                .padding(.vertical, KineticSpacing.sm)
            }
            .background(KineticColor.surfaceContainerHigh.opacity(0.8))
        }
        .background(KineticColor.surface)
        .onAppear {
            guard !isDisconnected else { return }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                keyboardFocused = true
            }
        }
        .onDisappear {
            keyboardFocused = false
        }
    }

    private var isDisconnected: Bool {
        if sessionHealth.isDisconnected { return true }
        if case .connectionLost = monitor.status { return true }
        if case .lost = monitor.transportHealth { return true }
        return false
    }

    private var statusSubtitle: String? {
        let base = monitor.lastHost.map { "Attached to \($0)" }
        let session = sessionStatusSummary
        let transport = transportSummary
        let reconnect = reconnectSummary
        let joined = [base, session, client.handshakeSummary, transport, reconnect].compactMap { $0 }.joined(separator: " · ")
        return joined.isEmpty ? nil : joined
    }

    private var disconnectReason: String? {
        if let sessionHealthIssue {
            return sessionHealthIssue
        }
        if case .connectionLost(let reason) = monitor.status {
            return reason
        }
        if case .lost(let reason) = monitor.transportHealth {
            return reason
        }
        return nil
    }

    private var transportSummary: String? {
        if !sessionHealth.allowsTransportSummary {
            return nil
        }
        switch monitor.transportHealth {
        case .idle:
            return nil
        case .healthy:
            return "transport healthy"
        case .degraded(let reason):
            return "transport degraded: \(reason)"
        case .lost(let reason):
            return "transport lost: \(reason)"
        }
    }

    private var sessionStatusSummary: String? {
        sessionHealth.statusSummary
    }

    private var reconnectSummary: String? {
        switch monitor.reconnectState {
        case .idle:
            return nil
        case .waiting(let attempt, _):
            return "reconnecting (\(attempt))"
        case .failed(let reason):
            return "reconnect failed: \(reason)"
        }
    }

    private func transportBanner(reason: String, color: Color) -> some View {
        Text(reason)
            .font(KineticFont.caption)
            .foregroundStyle(color)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(KineticSpacing.md)
            .background(color.opacity(0.1))
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

    private func sendTypedText(_ text: String) {
        guard !text.isEmpty else { return }

        if ctrlActive, text.count == 1, let first = text.first, first.isLetter,
           let ascii = first.uppercased().first?.asciiValue
        {
            client.sendInputBytes(Data([ascii - 64]))
            ctrlActive = false
            altActive = false
            metaActive = false
            return
        }

        if altActive || metaActive {
            client.sendInputBytes(Data([0x1b]))
            altActive = false
            metaActive = false
        }

        if text == "\r" {
            client.sendInputBytes(Data([0x0d]))
            return
        }

        client.sendInput(text)
    }

    private func handleTerminalGesture(_ action: RemoteTerminalGestureAction) {
        switch action {
        case .pageUp:
            sendSpecialKey([0x1b, 0x5b, 0x35, 0x7e])
        case .pageDown:
            sendSpecialKey([0x1b, 0x5b, 0x36, 0x7e])
        case .arrowLeft:
            sendSpecialKey([0x1b, 0x5b, 0x44])
        case .arrowRight:
            sendSpecialKey([0x1b, 0x5b, 0x43])
        }
    }

    private func sendSpecialKey(_ bytes: [UInt8]) {
        var payload = Data()
        if altActive || metaActive {
            payload.append(0x1b)
            altActive = false
            metaActive = false
        }
        if ctrlActive, bytes.count == 1, let ascii = asciiControlByte(for: bytes[0]) {
            payload.append(ascii)
            ctrlActive = false
        } else {
            payload.append(contentsOf: bytes)
        }
        client.sendInputBytes(payload)
    }

    private func asciiControlByte(for byte: UInt8) -> UInt8? {
        switch byte {
        case 0x69, 0x49:
            return 0x09
        case 0x6d, 0x4d:
            return 0x0d
        case 0x5b:
            return 0x1b
        default:
            return nil
        }
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
    @ObservedObject var client: GSPClient
    @ObservedObject var store: ConnectionStore
    @ObservedObject var tailscaleBrowser: TailscalePeerBrowser
    @ObservedObject var monitor: ConnectionMonitor
    @Binding var serverIdentityWarning: String?

    @State private var nodeName = ""
    @State private var nodeHost = ""
    @State private var nodePort = "7337"
    @State private var nodeAuthKey = ""
    @State private var tailscalePort = "7337"
    @State private var tailscaleToken = ""

    private var trustedIdentityRow: (current: String, trusted: String?)? {
        guard let host = monitor.lastHost,
              let port = monitor.lastPort,
              let current = client.serverIdentityId ?? client.lastSeenServerIdentityId,
              !current.isEmpty else { return nil }
        return (current, store.trustedServerIdentity(host: host, port: port))
    }

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(title: "Settings", subtitle: "Manage saved nodes and current connection state.")
            ScrollView {
                VStack(alignment: .leading, spacing: KineticSpacing.xl) {
                    if let serverIdentityWarning,
                       let trustedIdentityRow,
                       let host = monitor.lastHost,
                       let port = monitor.lastPort
                    {
                        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                            KineticSectionLabel(text: "Server Identity")
                            Text(serverIdentityWarning)
                                .font(KineticFont.caption)
                                .foregroundStyle(KineticColor.error)
                                .padding(KineticSpacing.md)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(KineticColor.error.opacity(0.1))
                                .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                            Text("Current: \(trustedIdentityRow.current)")
                                .font(KineticFont.caption)
                                .foregroundStyle(KineticColor.onSurfaceVariant)
                            if let trusted = trustedIdentityRow.trusted {
                                Text("Trusted: \(trusted)")
                                    .font(KineticFont.caption)
                                    .foregroundStyle(KineticColor.onSurfaceVariant)
                            }
                            Button("Trust Current Server Identity") {
                                store.trustServerIdentity(host: host, port: port, identityId: trustedIdentityRow.current)
                                self.serverIdentityWarning = nil
                            }
                            .buttonStyle(KineticPrimaryButtonStyle())
                        }
                    }

                    VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                        KineticSectionLabel(text: "Save Node")
                        KineticInputField(placeholder: "Name", text: $nodeName, accessibilityIdentifier: "settings-node-name-input")
                        KineticInputField(placeholder: "Host", text: $nodeHost, accessibilityIdentifier: "settings-node-host-input")
                        KineticInputField(placeholder: "Port", text: $nodePort, keyboardType: .numberPad, accessibilityIdentifier: "settings-node-port-input")
                        KineticInputField(placeholder: "Auth Key", text: $nodeAuthKey, secure: true, accessibilityIdentifier: "settings-node-authkey-input")
                        Button("Save Node") { saveNode() }
                            .buttonStyle(KineticPrimaryButtonStyle())
                            .accessibilityIdentifier("save-node-button")
                    }

                    VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                        KineticSectionLabel(text: "Tailscale Discovery")
                        Text("Use a Tailscale API access token to list tailnet devices. This does not reuse the Tailscale app session, and it does not verify that Boo is actually running on those devices.")
                            .font(KineticFont.caption)
                            .foregroundStyle(KineticColor.onSurfaceVariant)
                        if store.hasTailscaleAPIToken {
                            Text("API access token saved securely in the iOS Keychain.")
                                .font(KineticFont.caption)
                                .foregroundStyle(KineticColor.primary)
                        }
                        KineticInputField(placeholder: "Default Boo Port", text: $tailscalePort, keyboardType: .numberPad, accessibilityIdentifier: "settings-tailscale-port-input")
                        KineticInputField(placeholder: store.hasTailscaleAPIToken ? "Replace saved Tailscale API access token" : "Tailscale API Access Token", text: $tailscaleToken, accessibilityIdentifier: "settings-tailscale-token-input")
                        Button("Save Tailscale Settings") {
                            let port = UInt16(tailscalePort) ?? 7337
                            store.updateTailscaleDiscovery(defaultPort: port)
                            if !tailscaleToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                store.replaceTailscaleAPIToken(tailscaleToken)
                                tailscaleToken = ""
                            }
                            tailscaleBrowser.refresh(store: store)
                        }
                        .buttonStyle(KineticPrimaryButtonStyle())
                        .accessibilityIdentifier("save-tailscale-settings-button")
                        if store.hasTailscaleAPIToken {
                            Button("Clear Saved Tailscale Token") {
                                store.clearTailscaleAPIToken()
                                tailscaleToken = ""
                                tailscaleBrowser.refresh(store: store)
                            }
                            .buttonStyle(KineticSecondaryButtonStyle())
                            .accessibilityIdentifier("clear-tailscale-token-button")
                        }
                    }

                    if !store.savedNodes.isEmpty {
                        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                            KineticSectionLabel(text: "Saved Nodes")
                            ForEach(store.savedNodes) { node in
                                KineticCardRow(
                                    icon: "server.rack",
                                    title: node.name,
                                    subtitle: "\(node.host):\(node.port)",
                                    accessibilityIdentifier: "settings-saved-node-\(node.name)"
                                )
                            }
                        }
                    }

                    Button("Disconnect") { monitor.disconnect() }
                        .buttonStyle(KineticSecondaryButtonStyle())
                        .accessibilityIdentifier("settings-disconnect-button")

                    Button("Clear History") { store.clearHistory() }
                        .buttonStyle(KineticSecondaryButtonStyle())
                        .accessibilityIdentifier("clear-history-button")

                    Spacer().frame(height: 120)
                }
                .padding(.horizontal, KineticSpacing.md)
            }
        }
        .onAppear {
            tailscalePort = "\(store.tailscaleDiscoverySettings.defaultPort)"
            tailscaleToken = ""
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
