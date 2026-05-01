import SwiftUI
import UIKit
import Foundation

private func formatConnectionTarget(host: String, port: UInt16) -> String {
    port == BooDefaultRemotePort ? host : "\(host):\(port)"
}

private struct DashboardProbeMetrics: Equatable {
    let status: TailscalePeerProbeStatus
    let latencyMs: Double?
}

@MainActor
private final class DashboardProbeMonitor: ObservableObject {
    @Published private(set) var metrics: [String: DashboardProbeMetrics] = [:]

    private var tasks: [String: Task<Void, Never>] = [:]

    struct Target: Equatable {
        let key: String
        let host: String
        let port: UInt16
    }

    func updateTargets(_ targets: [Target]) {
        let targetKeys = Set(targets.map(\.key))

        for (key, task) in tasks where !targetKeys.contains(key) {
            task.cancel()
            tasks.removeValue(forKey: key)
            metrics.removeValue(forKey: key)
        }

        for target in targets {
            guard tasks[target.key] == nil else { continue }
            tasks[target.key] = Task { [weak self] in
                await self?.runProbeLoop(for: target)
            }
        }
    }

    func stop() {
        tasks.values.forEach { $0.cancel() }
        tasks.removeAll()
        metrics.removeAll()
    }

    private func runProbeLoop(for target: Target) async {
        var attempts = 0
        var consecutiveFailures = 0
        while !Task.isCancelled {
            let latency = await measureBooQUICHandshakeLatency(host: target.host, port: target.port)
            if Task.isCancelled { return }
            attempts += 1
            if latency == nil {
                consecutiveFailures += 1
            } else {
                consecutiveFailures = 0
            }
            await MainActor.run {
                let status: TailscalePeerProbeStatus
                if let latency {
                    status = .reachable
                } else if attempts > 0 && consecutiveFailures > 0 {
                    status = .unreachable
                } else {
                    status = .probing
                }
                self.metrics[target.key] = DashboardProbeMetrics(status: status, latencyMs: latency)
            }
            try? await Task.sleep(for: .seconds(15))
        }
    }
}

struct BooRootView: View {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var client = GSPClient()
    @StateObject private var tailscaleBrowser = TailscalePeerBrowser()
    @StateObject private var store = ConnectionStore()
    @State private var selectedTab: BooTab = .terminal
    @State private var showingConnectedTerminal = false
    @State private var monitor: ConnectionMonitor?
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
                switch selectedTab {
                case .terminal:
                    ZStack {
                        ConnectScreen(
                            client: client,
                            tailscaleBrowser: tailscaleBrowser,
                            store: store,
                            monitor: activeMonitor,
                            selectedTab: $selectedTab,
                            onPresentConnectedTerminal: {
                                showingConnectedTerminal = true
                            }
                        )
                        .opacity(showingConnectedTerminal ? 0 : 1)
                        .allowsHitTesting(!showingConnectedTerminal)
                        .accessibilityHidden(showingConnectedTerminal)
                    if showingConnectedTerminal {
                        TerminalTabScreen(
                            client: client,
                            monitor: activeMonitor,
                            store: store,
                                onBack: {
                                    showingConnectedTerminal = false
                                }
                            )
                        .zIndex(1)
                    }
                }
                    .ignoresSafeArea(showingConnectedTerminal ? [.container, .keyboard] : [], edges: .all)
                case .history:
                    HistoryScreen(store: store)
                case .settings:
                    SettingsScreen(
                        store: store,
                        tailscaleBrowser: tailscaleBrowser
                    )
                }
            }
            if !(selectedTab == .terminal && showingConnectedTerminal) {
                KineticTabBar(selectedTab: $selectedTab)
                    .padding(.bottom, KineticSpacing.md)
            }
        }
        .background(KineticColor.surface)
        .statusBarHidden(showingConnectedTerminal)
        .onAppear {
            if monitor == nil {
                monitor = ConnectionMonitor(client: client, store: store)
            }
            applyUITestLaunchConfigurationIfNeeded()
        }
        .onChange(of: activeMonitor.status) { oldValue, newValue in
            handleStatusChange(from: oldValue, to: newValue)
        }
        .onChange(of: client.activeTabId) { _, _ in
            returnToConnectionsIfCurrentTerminalClosed()
        }
        .onChange(of: client.runtimeState) { _, _ in
            returnToConnectionsIfCurrentTerminalClosed()
        }
        .onChange(of: client.lastErrorKind) { _, _ in
            returnToConnectionsIfCurrentTerminalClosed()
        }
        .onChange(of: scenePhase) { _, newPhase in
            guard newPhase == .active else { return }
            store.refreshTailscaleTokenStatus()
            tailscaleBrowser.refresh(store: store)
            guard activeMonitor.lastHost != nil, !client.connected else { return }
            activeMonitor.reconnect()
        }
        .onChange(of: client.lastError) { _, newValue in
            guard newValue != nil else { return }
        }
    }

    private func handleStatusChange(from oldValue: ConnectionStatus, to newValue: ConnectionStatus) {
        let wasConnected: Bool = {
            switch oldValue {
            case .connected, .authenticated, .activeTab:
                return true
            default:
                return false
            }
        }()
        switch newValue {
        case .authenticated, .activeTab:
            if selectedTab == .terminal {
                showingConnectedTerminal = true
            }
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

    private var currentTerminalHealth: ActiveTabHealth {
        resolveActiveTabHealth(
            activeTabId: client.activeTabId,
            tabs: client.tabs,
            authenticated: client.authenticated,
            runtimeViewId: client.runtimeState?.viewId,
            runtimeTabCount: client.runtimeState?.tabs.count,
            runtimeTabs: client.runtimeState?.tabs ?? [],
            lastErrorKind: client.lastErrorKind
        )
    }

    private func returnToConnectionsIfCurrentTerminalClosed() {
        guard showingConnectedTerminal else { return }
        switch currentTerminalHealth {
        case .expired, .exited:
            showingConnectedTerminal = false
            client.clearErrorState()
        case .opening, .detached, .unreachable, .reachable:
            break
        }
    }

    private func applyUITestLaunchConfigurationIfNeeded() {
        guard !didApplyUITestLaunchConfiguration else { return }
        didApplyUITestLaunchConfiguration = true
        guard let config = UITestLaunchConfiguration.current(),
              (config.forceActiveTerminal || config.forceOpeningTerminal || config.autoConnect)
        else { return }

        if config.forceActiveTerminal {
            client.disconnect()
            client.connected = true
            client.authenticated = true
            client.activeTabId = 1
            client.tabs = [
                RemoteTabInfo(
                    id: 1,
                    name: "Tab 1",
                    title: "Tab 1",
                    pwd: "~",
                    active: true,
                    childExited: false
                )
            ]
            client.runtimeState = RemoteRuntimeStateSnapshot(
                activeTab: 0,
                focusedPane: 1,
                tabs: [
                    RemoteRuntimeTabSnapshot(
                        tabId: 1,
                        index: 0,
                        active: true,
                        title: "Tab 1",
                        paneCount: 1,
                        focusedPane: 1,
                        paneIds: [1]
                    )
                ],
                visiblePanes: [
                    RemoteRuntimePaneSnapshot(
                        leafIndex: 0,
                        leafId: 1,
                        paneId: 1,
                        focused: true,
                        frame: RemoteRuntimeRectSnapshot(x: 0, y: 0, width: 320, height: 240),
                        splitDirection: nil,
                        splitRatio: nil
                    )
                ],
                pwd: "~",
                runtimeRevision: 1,
                viewRevision: 1,
                viewId: 1,
                viewedTabId: 1,
                viewportCols: 80,
                viewportRows: 24,
                visiblePaneIds: [1],
                ackedClientActionId: nil
            )
            client.paneScreens = [
                1: DecodedWireScreenState(
                    rows: 1,
                    cols: 1,
                    cells: [DecodedWireCell()],
                    cursorX: 0,
                    cursorY: 0,
                    cursorVisible: true,
                    cursorBlinking: false,
                    cursorStyle: 0
                )
            ]
            client.lastError = nil
            client.lastErrorKind = nil
            showingConnectedTerminal = true
            return
        }

        if config.forceOpeningTerminal {
            client.disconnect()
            client.connected = true
            client.authenticated = true
            client.activeTabId = nil
            client.tabs = []
            client.runtimeState = nil
            client.screen = ScreenState()
            client.paneScreens = [:]
            client.lastError = nil
            client.lastErrorKind = nil
            showingConnectedTerminal = true
            return
        }

        guard let host = config.host else { return }

        let matchingNodeId = store.savedNodes.first {
            $0.host == host && $0.port == config.port
        }?.id
        let historyId = store.recordConnection(
            nodeName: config.nodeName ?? host,
            host: formatConnectionTarget(host: host, port: config.port)
        )
        activeMonitor.connect(
            host: host,
            port: config.port,
            displayName: config.nodeName ?? host,
            historyId: historyId,
            nodeId: matchingNodeId
        )
    }

}

struct ConnectScreen: View {
    @ObservedObject var client: GSPClient
    @ObservedObject var tailscaleBrowser: TailscalePeerBrowser
    @ObservedObject var store: ConnectionStore
    @ObservedObject var monitor: ConnectionMonitor
    @Binding var selectedTab: BooTab
    let onPresentConnectedTerminal: () -> Void

    @StateObject private var dashboardProbeMonitor = DashboardProbeMonitor()
    @State private var host = ""
    @State private var didApplyUITestHostPrefill = false

    private var displayedConnectError: String? {
        guard let error = client.lastError else { return nil }
        let contextual = monitor.contextualErrorMessage(error)
        if error == "Connection timed out",
           let host = monitor.lastHost,
           let peer = tailscaleBrowser.peers.first(where: { $0.host == host || $0.address == host })
        {
            return "Timed out reaching \(peer.name) over Tailscale. Make sure this iPad is connected to Tailscale and Boo is listening on port \(peer.port)."
        }
        return contextual
    }

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
            return (decorateStatusMessage(reason), KineticColor.tertiary)
        case .lost(let reason):
            return (decorateStatusMessage(reason), KineticColor.error)
        default:
            break
        }
        switch monitor.status {
        case .connecting:
            return (decorateStatusMessage("Connecting…"), KineticColor.primary)
        case .connectionLost(let reason):
            return (decorateStatusMessage(reason), KineticColor.error)
        default:
            if !monitor.networkPathState.isSatisfied {
                return (monitor.networkStatusSummary, KineticColor.tertiary)
            }
            return nil
        }
    }

    var body: some View {
        ScrollView {
            scrollContent
        }
        .accessibilityIdentifier("connect-screen")
        .safeAreaInset(edge: .bottom) {
            Color.clear.frame(height: 96)
        }
        .onAppear {
            applyUITestHostPrefillIfNeeded()
            store.refreshTailscaleTokenStatus()
            tailscaleBrowser.refresh(store: store)
            refreshDashboardProbes()
        }
        .onChange(of: store.tailscaleDiscoverySettings) { _, _ in
            tailscaleBrowser.refresh(store: store)
        }
        .onReceive(store.$savedNodes) { _ in
            refreshDashboardProbes()
        }
    }

    private var scrollContent: some View {
        VStack(alignment: .leading, spacing: KineticSpacing.xl) {
            statusSection
            addressSection
            errorSection
            actionSection
            savedNodesSection
            tailscaleSection
            Spacer().frame(height: 120)
        }
        .padding(.horizontal, KineticSpacing.md)
    }

    @ViewBuilder
    private var statusSection: some View {
        if let statusBanner {
            Text(statusBanner.message)
                .font(KineticFont.caption)
                .foregroundStyle(statusBanner.color)
                .accessibilityIdentifier("connect-status-banner")
                .padding(KineticSpacing.md)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(statusBanner.color.opacity(0.1))
                .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
        }
    }

    private var addressSection: some View {
        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
            KineticSectionLabel(text: "Machine Address")
            KineticInputField(placeholder: "hostname or ip:port", text: $host, accessibilityIdentifier: "connect-host-input")
            Text("Connect directly to a LAN host, a Tailscale IP, or any other reachable Boo endpoint.")
                .font(KineticFont.caption)
                .foregroundStyle(KineticColor.onSurfaceVariant)
        }
    }

    @ViewBuilder
    private var errorSection: some View {
        if let error = displayedConnectError {
            Text(error)
                .font(KineticFont.caption)
                .foregroundStyle(KineticColor.error)
                .accessibilityIdentifier("connect-error-label")
            .padding(KineticSpacing.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(KineticColor.error.opacity(0.1))
            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
        }
    }

    private var actionSection: some View {
        VStack(spacing: KineticSpacing.sm) {
            Button("Connect") { connectManual() }
                .buttonStyle(KineticPrimaryButtonStyle())
                .disabled(host.isEmpty)
                .accessibilityIdentifier("connect-button")
        }
    }

    @ViewBuilder
    private var savedNodesSection: some View {
        if !store.savedNodes.isEmpty {
            VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                KineticSectionLabel(text: "Saved Nodes")
                ForEach(store.savedNodes) { node in
                    KineticCardRow(
                        icon: "server.rack",
                        title: node.name,
                        subtitle: rowSubtitle(base: "\(node.host):\(node.port)", host: node.host, port: node.port, nodeName: node.name),
                        trailingText: liveMetrics(host: node.host, port: node.port, nodeName: node.name),
                        trailingAccessibilityIdentifier: rowMetricAccessibilityIdentifier(nodeName: node.name),
                        onTap: {
                            let historyId = store.recordConnection(
                                nodeName: node.name,
                                host: formatConnectionTarget(host: node.host, port: node.port)
                            )
                            monitor.connect(
                                host: node.host,
                                port: node.port,
                                displayName: node.name,
                                historyId: historyId,
                                nodeId: node.id
                            )
                        },
                        accessibilityIdentifier: "saved-node-\(node.name)"
                    )
                }
            }
        }
    }

    private var tailscaleSection: some View {
        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
            KineticSectionLabel(text: "Tailscale Devices")
            Text("Devices in your tailnet from the Tailscale API. Boo still needs to be running on the configured port, and this iPad must be connected to Tailscale to reach them. Without embedded Tailscale, Boo iOS cannot run true `tailscale ping`, so these rows use a Boo-port probe instead.")
                .font(KineticFont.caption)
                .foregroundStyle(KineticColor.onSurfaceVariant)
            if !store.hasTailscaleAPIToken {
                Text("No Tailscale API token saved. Add one in Settings to list tailnet devices.")
                    .font(KineticFont.caption)
                    .foregroundStyle(KineticColor.onSurfaceVariant)
                    .accessibilityIdentifier("tailscale-token-missing-label")
            }
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
                let metrics = tailscaleBrowser.probeMetrics[peer.id]
                let canConnect = tailscalePeerCanConnect(peer, metrics: metrics)
                let detail = tailscalePeerDetail(peer, metrics: metrics)
                KineticCardRow(
                    icon: "network",
                    title: peer.name,
                    subtitle: rowSubtitle(base: detail, host: peer.host, port: peer.port, nodeName: peer.name),
                    trailingText: liveMetrics(host: peer.host, port: peer.port, nodeName: peer.name),
                    trailingAccessibilityIdentifier: rowMetricAccessibilityIdentifier(nodeName: peer.name),
                    subtitleAccessoryText: tailscalePortStatusText(peer, metrics: metrics),
                    subtitleAccessoryColor: tailscalePortStatusColor(metrics),
                    onTap: canConnect ? {
                        connectToHost(peer.host, port: peer.port, nodeName: peer.name, routeKind: .tailscale)
                    } : nil,
                    accessibilityIdentifier: "tailscale-peer-\(peer.name)"
                )
                .opacity(canConnect ? 1.0 : 0.6)
            }
        }
    }

    private func connectManual() {
        guard !host.isEmpty else { return }
        dismissConnectScreenKeyboard()
        let parsed = parseHost(host)
        connectToHost(parsed.0, port: parsed.1, nodeName: parsed.0, routeKind: .manual)
    }

    private func applyUITestHostPrefillIfNeeded() {
        guard !didApplyUITestHostPrefill else { return }
        didApplyUITestHostPrefill = true
        guard
            host.isEmpty,
            let config = UITestLaunchConfiguration.current(),
            let configuredHost = config.host
        else {
            return
        }
        host = formatConnectionTarget(host: configuredHost, port: config.port)
    }

    private func connectToHost(_ host: String, port: UInt16, nodeName: String, routeKind: ConnectionRouteKind = .manual) {
        dismissConnectScreenKeyboard()
        if shouldReuseActiveConnection(host: host, port: port, nodeName: nodeName) {
            DispatchQueue.main.async {
                onPresentConnectedTerminal()
            }
            return
        }
        let historyId = store.recordConnection(
            nodeName: nodeName,
            host: formatConnectionTarget(host: host, port: port)
        )
        monitor.connect(host: host, port: port, routeKind: routeKind, displayName: nodeName, historyId: historyId)
    }

    private func dismissConnectScreenKeyboard() {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )
    }

    private func decorateStatusMessage(_ raw: String) -> String {
        let contextual = monitor.contextualErrorMessage(raw)
        if let metrics = monitor.latencyAndLossSummary,
           !contextual.contains("loss"),
           contextual != "Connecting…" {
            return "\(contextual) · \(metrics)"
        }
        return contextual
    }

    private func shouldReuseActiveConnection(host: String, port: UInt16, nodeName: String) -> Bool {
        let sameEndpoint = monitor.lastHost == host && monitor.lastPort == port
        let sameDisplayName = monitor.lastDisplayName == nodeName && monitor.lastPort == port
        guard sameEndpoint || sameDisplayName else { return false }
        guard client.activeTabId != nil else {
            return false
        }
        switch monitor.status {
        case .connecting, .connected, .authenticated, .activeTab:
            return true
        case .connectionLost:
            return false
        case .disconnected:
            return false
        }
    }

    private func parseHost(_ raw: String) -> (String, UInt16) {
        if let index = raw.lastIndex(of: ":"), let port = UInt16(raw[raw.index(after: index)...]) {
            return (String(raw[..<index]), port)
        }
        return (raw, BooDefaultRemotePort)
    }

    private func tailscalePeerDetail(_ peer: TailscalePeer, metrics: TailscalePeerProbeMetrics?) -> String {
        var parts: [String] = []
        if let os = peer.os { parts.append(os) }
        parts.append(peer.stateDescription)
        if let address = peer.address { parts.append(address) }
        if metrics == nil {
            parts.append("boo:\(peer.port)")
        }
        if client.lastError == "Connection timed out",
           let host = monitor.lastHost,
           (peer.host == host || peer.address == host)
        {
            parts.append("unreachable from this iPad")
        }
        if let metrics = tailscaleBrowser.probeMetrics[peer.id],
           metrics.hostStatus == .reachable,
           let loss = metrics.lossRate,
           loss > 0 {
            parts.append("\(String(format: "%.0f", loss))% loss")
        }
        return parts.joined(separator: " · ")
    }

    private func liveMetrics(host: String, port: UInt16, nodeName: String) -> String? {
        if isCurrentTarget(host: host, port: port, nodeName: nodeName),
           let latency = monitor.latencyMs {
            return String(format: "%.0f ms", latency)
        }
        if let peer = tailscaleBrowser.peers.first(where: { $0.name == nodeName && $0.port == port }) {
            if let metrics = tailscaleBrowser.probeMetrics[peer.id] {
                switch metrics.hostStatus {
                case .reachable:
                    if let latency = metrics.latencyMs {
                        return String(format: "%.0f ms", latency)
                    }
                case .probing:
                    return "probing"
                case .unreachable:
                    return "unreachable"
                }
            } else if peer.online {
                return "probing"
            }
        }
        let dashboardKey = dashboardProbeKey(host: host, port: port, nodeName: nodeName)
        if let metrics = dashboardProbeMonitor.metrics[dashboardKey] {
            switch metrics.status {
            case .reachable:
                if let latency = metrics.latencyMs {
                    return String(format: "%.0f ms", latency)
                }
            case .probing:
                return "probing"
            case .unreachable:
                return "unreachable"
            }
        }
        return nil
    }

    private func tailscalePeerCanConnect(_ peer: TailscalePeer, metrics: TailscalePeerProbeMetrics?) -> Bool {
        guard peer.online else { return false }
        guard let metrics else { return true }
        guard metrics.hostStatus != .unreachable else { return false }
        guard metrics.portStatus != .closed else { return false }
        return true
    }

    private func tailscalePortStatusText(_ peer: TailscalePeer, metrics: TailscalePeerProbeMetrics?) -> String? {
        guard metrics != nil else { return nil }
        return ":\(peer.port)"
    }

    private func tailscalePortStatusColor(_ metrics: TailscalePeerProbeMetrics?) -> Color {
        guard let metrics else {
            return KineticColor.tertiary
        }
        switch metrics.portStatus {
        case .probing:
            return KineticColor.tertiary
        case .open:
            return KineticColor.success
        case .closed:
            return KineticColor.error
        }
    }

    private func rowSubtitle(base: String, host: String, port: UInt16, nodeName: String) -> String {
        guard isCurrentTarget(host: host, port: port, nodeName: nodeName) else { return base }
        guard let loss = monitor.estimatedPacketLossRate, loss > 0 else { return base }
        return "\(base) · \(String(format: "%.0f%% loss", loss))"
    }

    private func rowMetricAccessibilityIdentifier(nodeName: String) -> String {
        "host-metric-\(nodeName)"
    }

    private func dashboardProbeKey(host: String, port: UInt16, nodeName: String) -> String {
        "\(nodeName)|\(host)|\(port)"
    }

    private func refreshDashboardProbes() {
        let savedTargets = store.savedNodes.map { node in
            DashboardProbeMonitor.Target(
                key: dashboardProbeKey(host: node.host, port: node.port, nodeName: node.name),
                host: node.host,
                port: node.port
            )
        }
        dashboardProbeMonitor.updateTargets(savedTargets)
    }

    private func isCurrentTarget(host: String, port: UInt16, nodeName: String) -> Bool {
        guard monitor.lastPort == port else { return false }
        if monitor.lastHost == host {
            return true
        }
        if monitor.lastDisplayName == nodeName {
            return true
        }
        return false
    }
}

private enum TerminalModifierState {
    case inactive
    case held
    case latched

    var isActive: Bool {
        switch self {
        case .inactive:
            return false
        case .held, .latched:
            return true
        }
    }
}

private enum UITestTraceAutomationStep {
    case idle
    case requestedRuntimeViewE2ENewTab
    case requestedRuntimeViewE2ESetViewedTab(tabId: UInt32)
    case requestedRuntimeViewE2ESplitRight
    case requestedRuntimeViewE2ESplitDown
    case requestedRuntimeViewE2EFocus(paneId: UInt64)
    case requestedInitialSetViewedTab(tabId: UInt32)
    case requestedSplit
    case requestedFocus(paneId: UInt64)
    case focusDone
    case requestedNewTab(originalTabId: UInt32?)
    case requestedSetViewedTab(tabId: UInt32)
    case setViewedTabDone
    case sentInput
    case done

    var debugName: String {
        switch self {
        case .idle:
            return "idle"
        case .requestedRuntimeViewE2ENewTab:
            return "requestedRuntimeViewE2ENewTab"
        case .requestedRuntimeViewE2ESetViewedTab(let tabId):
            return "requestedRuntimeViewE2ESetViewedTab:\(tabId)"
        case .requestedRuntimeViewE2ESplitRight:
            return "requestedRuntimeViewE2ESplitRight"
        case .requestedRuntimeViewE2ESplitDown:
            return "requestedRuntimeViewE2ESplitDown"
        case .requestedRuntimeViewE2EFocus(let paneId):
            return "requestedRuntimeViewE2EFocus:\(paneId)"
        case .requestedInitialSetViewedTab(let tabId):
            return "requestedInitialSetViewedTab:\(tabId)"
        case .requestedSplit:
            return "requestedSplit"
        case .requestedFocus(let paneId):
            return "requestedFocus:\(paneId)"
        case .focusDone:
            return "focusDone"
        case .requestedNewTab(let originalTabId):
            return "requestedNewTab:\(originalTabId.map(String.init) ?? "nil")"
        case .requestedSetViewedTab(let tabId):
            return "requestedSetViewedTab:\(tabId)"
        case .setViewedTabDone:
            return "setViewedTabDone"
        case .sentInput:
            return "sentInput"
        case .done:
            return "done"
        }
    }
}

private struct TerminalViewportSize: Equatable {
    let cols: UInt16
    let rows: UInt16
}

struct TerminalTabScreen: View {
    @ObservedObject var client: GSPClient
    @ObservedObject var monitor: ConnectionMonitor
    @ObservedObject var store: ConnectionStore
    let onBack: () -> Void

    @State private var keyboardFocused = false
    @State private var ctrlModifierState: TerminalModifierState = .inactive
    @State private var altModifierState: TerminalModifierState = .inactive
    @State private var metaModifierState: TerminalModifierState = .inactive
    @State private var ctrlModifierConsumedWhileHeld = false
    @State private var altModifierConsumedWhileHeld = false
    @State private var metaModifierConsumedWhileHeld = false
    @State private var didApplyUITestForcedError = false
    @State private var didSendUITestTraceInput = false
    @State private var showingComposeInput = false
    @State private var composeText = ""
    @State private var terminalGestureStatus: String?
    @State private var terminalGestureStatusGeneration: UInt64 = 0
    @State private var uiTestTraceAutomationStep: UITestTraceAutomationStep = .idle
    @State private var didReturnToConnectionsForClosedTab = false
    @State private var openingTimeoutGeneration: UInt64 = 0
    @State private var openingTimeoutArmed = false
    @State private var keyboardOverlapHeight: CGFloat = 0

    private var tabHealth: ActiveTabHealth {
        resolveActiveTabHealth(
            activeTabId: client.activeTabId,
            tabs: client.tabs,
            authenticated: client.authenticated,
            runtimeViewId: client.runtimeState?.viewId,
            runtimeTabCount: client.runtimeState?.tabs.count,
            runtimeTabs: client.runtimeState?.tabs ?? [],
            lastErrorKind: client.lastErrorKind
        )
    }

    var body: some View {
        ZStack(alignment: .topLeading) {
            terminalTabBody
                .background(KineticColor.surface.ignoresSafeArea())
                .navigationBarBackButtonHidden(true)
                .toolbar(.hidden, for: .navigationBar)

            GeometryReader { geo in
                EdgeBackSwipeCapture {
                    goBack()
                }
                    .frame(width: min(max(geo.size.width * 0.14, 56), 104))
                    .accessibilityIdentifier("terminal-back-swipe-zone")
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)

        }
        .onAppear {
            applyUITestForcedErrorIfNeeded()
            client.attachView()
            returnToConnectionsIfTerminalClosed(tabHealth)
            updateOpeningTimeout(for: tabHealth)
            guard !isDisconnected, client.activeTabId != nil else { return }
            if !suppressesAutomaticKeyboardFocusForUITest {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                    requestKeyboardFocus()
                }
            }
            advanceUITestTraceAutomationIfNeeded()
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillChangeFrameNotification)) { notification in
            updateKeyboardOverlap(from: notification)
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { notification in
            keyboardFocused = false
            updateKeyboardOverlap(from: notification, forceHidden: true)
        }
        .onReceive(client.$tabs) { _ in
            applyUITestForcedErrorIfNeeded()
            advanceUITestTraceAutomationIfNeeded()
            updateOpeningTimeout(for: tabHealth)
        }
        .onReceive(client.$runtimeState) { _ in
            applyUITestForcedErrorIfNeeded()
            advanceUITestTraceAutomationIfNeeded()
            DispatchQueue.main.async {
                returnToConnectionsIfTerminalClosed(tabHealth)
                updateOpeningTimeout(for: tabHealth)
            }
        }
        .onChange(of: client.activeTabId) { _, activeTabId in
            if activeTabId != nil {
                invalidateOpeningTimeout()
                client.attachView()
                keyboardFocused = false
                terminalGestureStatus = nil
                didReturnToConnectionsForClosedTab = false
                if !suppressesAutomaticKeyboardFocusForUITest {
                    DispatchQueue.main.async {
                        requestKeyboardFocus()
                    }
                }
                applyUITestForcedErrorIfNeeded()
                advanceUITestTraceAutomationIfNeeded()
            } else {
                returnToConnectionsIfTerminalClosed(tabHealth)
                updateOpeningTimeout(for: tabHealth)
            }
        }
        .onChange(of: tabHealth) { _, newValue in
            returnToConnectionsIfTerminalClosed(newValue)
            updateOpeningTimeout(for: newValue)
        }
        .onChange(of: isDisconnected) { _, disconnected in
            if disconnected {
                keyboardFocused = false
            }
        }
        .sheet(isPresented: $showingComposeInput) {
            composeInputSheet
        }
        .onDisappear {
            invalidateOpeningTimeout()
            keyboardFocused = false
            keyboardOverlapHeight = 0
            client.detachView()
        }
    }

    private var terminalTabBody: some View {
        ZStack(alignment: .topLeading) {
            terminalView
                .overlay(alignment: .topTrailing) {
                    terminalGestureHUD
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            if UITestLaunchConfiguration.current() != nil {
                VStack(spacing: 0) {
                    Color.clear
                        .frame(width: 1, height: 1)
                        .accessibilityIdentifier("terminal-debug-state")
                        .accessibilityLabel(client.uiTestTabDebugSummary)
                    Color.clear
                        .frame(width: 1, height: 1)
                        .accessibilityIdentifier("terminal-trace-state")
                        .accessibilityLabel(uiTestTraceAutomationDebugSummary)
                }
                .allowsHitTesting(false)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .ignoresSafeArea([.container, .keyboard], edges: .all)
    }

    private var uiTestTraceAutomationDebugSummary: String {
        let marker = UITestLaunchConfiguration.current()?.traceOutputMarker ?? ""
        let outputObserved = !marker.isEmpty && client.runtimeAccessibilityTextSnapshot.contains(marker)
        return [
            "traceStep=\(uiTestTraceAutomationStep.debugName)",
            "traceInputSent=\(didSendUITestTraceInput)",
            "traceOutputObserved=\(outputObserved)",
        ].joined(separator: " ")
    }


    @ViewBuilder
    private var connectionHealthHUD: some View {
        if store.terminalDisplaySettings.showConnectionHealthHUD {
            HStack(spacing: KineticSpacing.sm) {
                Image(systemName: client.connected ? "antenna.radiowaves.left.and.right" : "wifi.slash")
                    .font(.system(size: 11, weight: .semibold))
                Text(client.connectionHealthSummary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
            }
            .font(KineticFont.caption)
            .foregroundStyle(KineticColor.onSurfaceVariant)
            .padding(.horizontal, KineticSpacing.md)
            .padding(.vertical, 4)
            .background(KineticColor.surfaceContainer.opacity(0.55))
            .accessibilityIdentifier("terminal-connection-health-hud")
            .accessibilityLabel(client.connectionHealthSummary)
        }
    }

    private var composeInputSheet: some View {
        NavigationStack {
            VStack(spacing: KineticSpacing.md) {
                TextEditor(text: $composeText)
                    .font(.system(size: 17, design: .monospaced))
                    .autocorrectionDisabled(false)
                    .textInputAutocapitalization(.never)
                    .scrollContentBackground(.hidden)
                    .padding(KineticSpacing.sm)
                    .background(KineticColor.surfaceContainer.opacity(0.45))
                    .clipShape(RoundedRectangle(cornerRadius: KineticRadius.container))
                    .accessibilityIdentifier("terminal-compose-editor")

                Text("Draft text with iOS input methods, then send it to the server-owned terminal state explicitly.")
                    .font(KineticFont.caption)
                    .foregroundStyle(KineticColor.onSurfaceVariant)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(KineticSpacing.lg)
            .background(KineticColor.surface.ignoresSafeArea())
            .navigationTitle("Compose Input")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        showingComposeInput = false
                    }
                    .accessibilityIdentifier("terminal-compose-cancel")
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Send") {
                        let text = composeText
                        composeText = ""
                        showingComposeInput = false
                        sendTypedText(text)
                    }
                    .disabled(composeText.isEmpty)
                    .accessibilityIdentifier("terminal-compose-send")
                }
            }
        }
    }

    @ViewBuilder
    private var runtimeOpeningOverlay: some View {
        if tabHealth == .opening {
            VStack(spacing: KineticSpacing.sm) {
                ProgressView()
                    .tint(KineticColor.primary)
                Text("Opening tab…")
                    .font(KineticFont.caption)
                    .foregroundStyle(KineticColor.onSurfaceVariant)
            }
            .padding(KineticSpacing.lg)
            .background(.ultraThinMaterial)
            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
            .accessibilityIdentifier("terminal-opening-overlay")
        }
    }

    @ViewBuilder
    private var terminalGestureHUD: some View {
        if let terminalGestureStatus {
            Text(terminalGestureStatus)
                .font(KineticFont.caption)
                .fontWeight(.semibold)
                .padding(.horizontal, KineticSpacing.sm)
                .padding(.vertical, KineticSpacing.xs)
                .background(KineticColor.surfaceContainerHigh.opacity(0.88))
                .foregroundStyle(KineticColor.onSurface)
                .clipShape(Capsule())
                .padding(.top, KineticSpacing.sm)
                .padding(.trailing, KineticSpacing.sm)
                .accessibilityIdentifier("terminal-gesture-status")
        }
    }

    private var terminalView: some View {
        let bridge = terminalKeyboardBridge

        return ZStack {
            bridge
                .id("terminal-keyboard-\(client.connectionDebugGeneration)-\(client.activeTabId ?? 0)")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .contentShape(Rectangle())
                .allowsHitTesting(false)

            if let runtimeState = client.runtimeState, !runtimeState.visiblePanes.isEmpty {
                GeometryReader { geo in
                    let layoutSize = terminalLayoutSize(from: geo)
                    let terminalSize = terminalSize(for: layoutSize)
                    ZStack(alignment: .topLeading) {
                        ForEach(runtimeState.visiblePanes, id: \.paneId) { pane in
                            ZStack {
                                Color.black.opacity(0.001)
                                RemoteTerminalCanvasView(
                                    state: client.displayedPaneScreen(paneId: pane.paneId),
                                    keyboardAvoidanceInset: keyboardAvoidanceInset(
                                        for: pane,
                                        terminalHeight: layoutSize.height
                                    ),
                                    scrollbackOffsetRows: client.displayedPaneScrollOffsetRows(paneId: pane.paneId),
                                    onGestureAction: { action in
                                        handleTerminalGesture(
                                            action,
                                            paneId: pane.paneId,
                                            viewedTabId: runtimeState.viewedTabId,
                                            isFocused: pane.paneId == runtimeState.focusedPane
                                        )
                                    }
                                )
                            }
                            .frame(
                                width: max(1, pane.frame.width),
                                height: max(1, pane.frame.height)
                            )
                            .overlay {
                                RoundedRectangle(cornerRadius: 8)
                                    .stroke(
                                        pane.paneId == runtimeState.focusedPane ? KineticColor.primary : KineticColor.onSurfaceVariant,
                                        lineWidth: pane.paneId == runtimeState.focusedPane ? 2 : 1
                                    )
                            }
                            .position(
                                x: pane.frame.x + pane.frame.width / 2,
                                y: pane.frame.y + pane.frame.height / 2
                            )
                            .contentShape(Rectangle())
                            .onTapGesture {
                                focusRuntimePane(pane.paneId, viewedTabId: runtimeState.viewedTabId)
                            }
                            .accessibilityElement(children: .ignore)
                            .accessibilityIdentifier("terminal-pane-\(pane.paneId)")
                            .accessibilityLabel("pane \(pane.paneId)\(pane.paneId == runtimeState.focusedPane ? " focused" : "")")
                            .accessibilityValue(client.paneAccessibilityText(paneId: pane.paneId))
                            .accessibilityAction {
                                focusRuntimePane(pane.paneId, viewedTabId: runtimeState.viewedTabId)
                            }
                        }

                        ForEach(Array(runtimePaneDividers(runtimeState.visiblePanes).enumerated()), id: \.offset) { _, divider in
                            Rectangle()
                                .fill(Color.clear)
                                .frame(width: divider.hitWidth, height: divider.hitHeight)
                                .position(x: divider.center.x, y: divider.center.y)
                                .contentShape(Rectangle())
                                .gesture(
                                    DragGesture(minimumDistance: 4)
                                        .onEnded { value in
                                            guard let ratio = divider.normalizedRatio(for: value.location) else { return }
                                            client.resizeSplit(
                                                direction: divider.runtimeDirection,
                                                ratio: ratio,
                                                optimisticFirstPaneId: divider.firstPaneId,
                                                optimisticSecondPaneId: divider.secondPaneId
                                            )
                                        }
                            )
                        }
                    }
                    .frame(width: layoutSize.width, height: layoutSize.height, alignment: .topLeading)
                    .background(.black)
                    .contentShape(Rectangle())
                    .onAppear {
                        client.setPreferredNewTabSize(cols: terminalSize.cols, rows: terminalSize.rows)
                        client.sendResize(cols: terminalSize.cols, rows: terminalSize.rows)
                    }
                    .onChange(of: terminalSize) { _, newSize in
                        client.setPreferredNewTabSize(cols: newSize.cols, rows: newSize.rows)
                        client.sendResize(cols: newSize.cols, rows: newSize.rows)
                    }
                    .highPriorityGesture(
                        SpatialTapGesture()
                            .onEnded { value in
                                focusRuntimePane(at: value.location, in: runtimeState)
                            }
                    )
                }
            } else {
                GeometryReader { geo in
                    let layoutSize = terminalLayoutSize(from: geo)
                    let terminalSize = terminalSize(for: layoutSize)
                    RemoteTerminalView(
                        screen: client.screen,
                        keyboardAvoidanceInset: keyboardOverlapHeight
                    ) { _, _ in
                        client.setPreferredNewTabSize(cols: terminalSize.cols, rows: terminalSize.rows)
                    } onGestureAction: { action in
                        handleTerminalGesture(action)
                    }
                    .frame(width: layoutSize.width, height: layoutSize.height, alignment: .topLeading)
                    .onAppear {
                        client.setPreferredNewTabSize(cols: terminalSize.cols, rows: terminalSize.rows)
                        client.sendResize(cols: terminalSize.cols, rows: terminalSize.rows)
                    }
                    .onChange(of: terminalSize) { _, newSize in
                        client.setPreferredNewTabSize(cols: newSize.cols, rows: newSize.rows)
                        client.sendResize(cols: newSize.cols, rows: newSize.rows)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture {
                    requestKeyboardFocus()
                }
            }
        }
        .opacity(isDisconnected || client.activeTabId == nil ? 0.5 : 1.0)
        .contentShape(Rectangle())
        .simultaneousGesture(
            TapGesture().onEnded {
                requestKeyboardFocus()
            }
        )
        .overlay {
            terminalAccessibilityOverlay
        }
        .overlay {
            runtimeOpeningOverlay
        }
    }

    private func updateKeyboardOverlap(from notification: Notification, forceHidden: Bool = false) {
        let nextOverlap: CGFloat
        if forceHidden {
            nextOverlap = 0
        } else {
            nextOverlap = keyboardOverlap(from: notification)
        }

        let duration = (notification.userInfo?[UIResponder.keyboardAnimationDurationUserInfoKey] as? NSNumber)?.doubleValue ?? 0.25
        let animation = Animation.easeOut(duration: max(0.01, duration))
        withAnimation(animation) {
            keyboardOverlapHeight = nextOverlap
        }
    }

    private func keyboardOverlap(from notification: Notification) -> CGFloat {
        guard let screenFrame = notification.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect,
              let windowScene = UIApplication.shared.connectedScenes
                  .compactMap({ $0 as? UIWindowScene })
                  .first(where: { $0.activationState == .foregroundActive }),
              let window = windowScene.windows.first(where: \.isKeyWindow)
        else {
            return 0
        }

        let frameInWindow = window.convert(screenFrame, from: nil)
        let intersection = window.bounds.intersection(frameInWindow)
        guard !intersection.isNull else { return 0 }
        return max(0, intersection.height - window.safeAreaInsets.bottom)
    }

    private func keyboardAvoidanceInset(
        for pane: RemoteRuntimePaneSnapshot,
        terminalHeight: CGFloat
    ) -> CGFloat {
        guard keyboardOverlapHeight > 0 else { return 0 }
        let keyboardTop = max(0, terminalHeight - keyboardOverlapHeight)
        let paneBottom = CGFloat(pane.frame.y + pane.frame.height)
        return max(0, paneBottom - keyboardTop)
    }

    private func terminalLayoutSize(from geo: GeometryProxy) -> CGSize {
        // The software keyboard is an overlay for Boo's mobile terminal, not a
        // terminal resize event. SwiftUI may shrink GeometryReader while the
        // keyboard is visible; add the measured keyboard overlap back so the
        // server-owned terminal grid and pane frames stay at the full view size.
        CGSize(
            width: max(1, geo.size.width),
            height: max(17, geo.size.height + keyboardOverlapHeight)
        )
    }

    private func terminalSize(for layoutSize: CGSize) -> TerminalViewportSize {
        return TerminalViewportSize(
            cols: max(1, UInt16(layoutSize.width / 8.4)),
            rows: max(1, UInt16(layoutSize.height / 17))
        )
    }

    private var terminalAccessibilityOverlay: some View {
        Color.clear
            .accessibilityElement(children: .ignore)
            .accessibilityIdentifier("terminal-screen")
            .accessibilityLabel(client.activeTabId.map { "active-\($0)" } ?? "inactive")
            .accessibilityValue(client.runtimeAccessibilityTextSnapshot)
            .allowsHitTesting(false)
    }

    private func runtimePaneDividers(_ panes: [RemoteRuntimePaneSnapshot]) -> [RuntimePaneDivider] {
        var dividers: [RuntimePaneDivider] = []
        for (index, first) in panes.enumerated() {
            for second in panes.dropFirst(index + 1) {
                if let divider = RuntimePaneDivider.make(first: first, second: second),
                   !dividers.contains(where: { $0.isSame(as: divider) }) {
                    dividers.append(divider)
                }
            }
        }
        return dividers
    }

    private func runtimePaneId(
        at location: CGPoint,
        in panes: [RemoteRuntimePaneSnapshot]
    ) -> UInt64? {
        panes.first { pane in
            let minX = CGFloat(pane.frame.x)
            let minY = CGFloat(pane.frame.y)
            let maxX = minX + CGFloat(pane.frame.width)
            let maxY = minY + CGFloat(pane.frame.height)
            return location.x >= minX &&
                location.x <= maxX &&
                location.y >= minY &&
                location.y <= maxY
        }?.paneId
    }

    private func focusRuntimePane(_ paneId: UInt64, viewedTabId: UInt32?) {
        guard let viewedTabId,
              let runtimeState = client.runtimeState,
              runtimeState.viewedTabId == viewedTabId,
              runtimeState.visiblePaneIds.contains(paneId)
        else { return }
        client.focusPane(tabId: viewedTabId, paneId: paneId)
        requestKeyboardFocus()
    }

    private func focusRuntimePane(at location: CGPoint, in runtimeState: RemoteRuntimeStateSnapshot) {
        guard let pane = runtimeState.visiblePanes.last(where: { pane in
            Double(location.x) >= pane.frame.x &&
                Double(location.x) <= pane.frame.x + pane.frame.width &&
                Double(location.y) >= pane.frame.y &&
                Double(location.y) <= pane.frame.y + pane.frame.height
        }) else {
            requestKeyboardFocus()
            return
        }
        focusRuntimePane(pane.paneId, viewedTabId: runtimeState.viewedTabId)
    }

    private func focusRuntimePaneForGesture(_ paneId: UInt64, viewedTabId: UInt32?) -> Bool {
        guard let viewedTabId,
              let runtimeState = client.runtimeState,
              runtimeState.viewedTabId == viewedTabId,
              runtimeState.visiblePaneIds.contains(paneId)
        else { return false }
        client.focusPane(tabId: viewedTabId, paneId: paneId)
        return true
    }

    private var suppressesAutomaticKeyboardFocusForUITest: Bool {
        UITestLaunchConfiguration.current()?.traceActions.contains("runtime-view-e2e") ?? false
    }

    private var openingTabTimeout: TimeInterval {
        if let override = UITestLaunchConfiguration.current()?.terminalOpeningTimeoutSeconds {
            return override
        }
        if UITestLaunchConfiguration.current()?.traceActions.contains("runtime-view-e2e") == true {
            return 60
        }
        return 12
    }

    private var terminalKeyboardBridge: some View {
        TerminalKeyboardBridge(
            isFocused: $keyboardFocused,
            onText: sendTypedText,
            onBackspace: {
                client.sendInputBytes(Data([0x7f]))
            },
            onKeyCommand: handleKeyCommand(input:modifiers:),
            accessoryState: terminalAccessoryState
        )
    }

    private func requestKeyboardFocus() {
        guard !isDisconnected, client.activeTabId != nil else { return }
        if keyboardFocused, keyboardOverlapHeight > 0 {
            return
        }
        if keyboardFocused {
            keyboardFocused = false
            DispatchQueue.main.async {
                guard !isDisconnected, client.activeTabId != nil else { return }
                keyboardFocused = true
            }
        } else {
            keyboardFocused = true
        }
    }

    private var terminalAccessoryState: TerminalKeyboardAccessoryState {
        TerminalKeyboardAccessoryState(
            ctrlActive: ctrlModifierState.isActive,
            altActive: altModifierState.isActive,
            metaActive: metaModifierState.isActive,
            onInsertText: sendTypedText,
            onEscape: { sendSpecialKey([0x1b]) },
            onCompose: { showingComposeInput = true },
            onCtrlModifierEvent: { handleModifierEvent($0, modifier: .ctrl) },
            onAltModifierEvent: { handleModifierEvent($0, modifier: .alt) },
            onMetaModifierEvent: { handleModifierEvent($0, modifier: .meta) },
            onFunctionKey: sendFunctionKey,
            onTab: { sendSpecialKey([0x09]) },
            onArrowUp: { sendSpecialKey([0x1b, 0x5b, 0x41]) },
            onArrowDown: { sendSpecialKey([0x1b, 0x5b, 0x42]) },
            onArrowLeft: { sendSpecialKey([0x1b, 0x5b, 0x44]) },
            onArrowRight: { sendSpecialKey([0x1b, 0x5b, 0x43]) },
            onPageUp: { sendSpecialKey([0x1b, 0x5b, 0x35, 0x7e]) },
            onPageDown: { sendSpecialKey([0x1b, 0x5b, 0x36, 0x7e]) },
            onHome: { sendSpecialKey([0x1b, 0x5b, 0x48]) },
            onEnd: { sendSpecialKey([0x1b, 0x5b, 0x46]) }
        )
    }

    private var isDisconnected: Bool {
        if tabHealth.isDisconnected { return true }
        if case .connectionLost = monitor.status { return true }
        if case .lost = monitor.transportHealth { return true }
        return false
    }

    private func returnToConnectionsIfTerminalClosed(_ health: ActiveTabHealth) {
        switch health {
        case .expired, .exited:
            guard !didReturnToConnectionsForClosedTab else { return }
            didReturnToConnectionsForClosedTab = true
            invalidateOpeningTimeout()
            keyboardFocused = false
            terminalGestureStatus = nil
            resetModifierStates()
            client.clearErrorState()
            DispatchQueue.main.async {
                onBack()
            }
        case .opening, .detached, .unreachable, .reachable:
            break
        }
    }

    private func goBack() {
        invalidateOpeningTimeout()
        keyboardFocused = false
        terminalGestureStatus = nil
        resetModifierStates()
        DispatchQueue.main.async {
            onBack()
        }
    }

    private func updateOpeningTimeout(for health: ActiveTabHealth) {
        guard health == .opening else {
            invalidateOpeningTimeout()
            return
        }
        guard !openingTimeoutArmed else { return }

        openingTimeoutArmed = true
        openingTimeoutGeneration &+= 1
        let generation = openingTimeoutGeneration
        DispatchQueue.main.asyncAfter(deadline: .now() + openingTabTimeout) {
            guard openingTimeoutArmed,
                  openingTimeoutGeneration == generation,
                  tabHealth == .opening
            else { return }
            handleOpeningTimeout()
        }
    }

    private func invalidateOpeningTimeout() {
        guard openingTimeoutArmed else { return }
        openingTimeoutArmed = false
        openingTimeoutGeneration &+= 1
    }

    private func handleOpeningTimeout() {
        guard tabHealth == .opening else {
            invalidateOpeningTimeout()
            return
        }
        invalidateOpeningTimeout()
        keyboardFocused = false
        terminalGestureStatus = nil
        resetModifierStates()
        client.lastErrorKind = .remote(openingTabTimeoutMessage)
        client.lastError = openingTabTimeoutMessage
        DispatchQueue.main.async {
            onBack()
        }
    }

    private var openingTabTimeoutMessage: String {
        let target = monitor.lastDisplayName ?? monitor.lastHost ?? "remote Boo server"
        return "Timed out opening a terminal tab on \(target). Make sure the Boo server is current and can start a shell."
    }

    private func applyUITestForcedErrorIfNeeded() {
        guard !didApplyUITestForcedError,
              client.activeTabId != nil,
              let rawKind = UITestLaunchConfiguration.current()?.forcedTerminalErrorKind,
              let kind = ClientWireErrorKind.uiTestNamed(rawKind)
        else { return }
        didApplyUITestForcedError = true
        client.lastErrorKind = kind
        client.lastError = kind.message
    }

    private func sendUITestTraceInputIfNeeded() {
        guard !didSendUITestTraceInput,
              client.activeTabId != nil,
              let command = UITestLaunchConfiguration.current()?.traceInputCommand,
              !command.isEmpty
        else { return }
        didSendUITestTraceInput = true

        var payload = Data(command.utf8)
        payload.append(0x0d)
        client.sendInputBytes(payload)
    }

    private func advanceUITestTraceAutomationIfNeeded() {
        guard let config = UITestLaunchConfiguration.current() else { return }
        let actions = config.traceActions
        if actions.isEmpty {
            sendUITestTraceInputIfNeeded()
            return
        }
        guard let runtimeState = client.runtimeState,
              runtimeState.viewId != 0
        else { return }

        if actions.contains("runtime-view-e2e"),
           advanceUITestRuntimeViewE2EIfNeeded(runtimeState) {
            return
        }

        guard client.activeTabId != nil else { return }

        switch uiTestTraceAutomationStep {
        case .idle:
            if let targetTabId = uiTestTargetViewedTabId(config, runtimeState),
               runtimeState.viewedTabId != targetTabId
            {
                uiTestTraceAutomationStep = .requestedInitialSetViewedTab(tabId: targetTabId)
                client.setViewedTab(targetTabId)
                return
            }
            if actions.contains("focus-pane") {
                if runtimeState.visiblePanes.count >= 2 {
                    requestUITestFocusPane(runtimeState)
                } else {
                    uiTestTraceAutomationStep = .requestedSplit
                    client.newSplit(direction: "right")
                }
                return
            }
            uiTestTraceAutomationStep = .focusDone
            advanceUITestTraceAutomationIfNeeded()

        case .requestedSplit:
            guard runtimeState.visiblePanes.count >= 2 else { return }
            requestUITestFocusPane(runtimeState)

        case .requestedFocus(let paneId):
            guard runtimeState.focusedPane == paneId else { return }
            uiTestTraceAutomationStep = .focusDone
            advanceUITestTraceAutomationIfNeeded()

        case .requestedInitialSetViewedTab(let tabId):
            guard runtimeState.viewedTabId == tabId else { return }
            uiTestTraceAutomationStep = .idle
            advanceUITestTraceAutomationIfNeeded()

        case .requestedRuntimeViewE2ENewTab,
             .requestedRuntimeViewE2ESetViewedTab,
             .requestedRuntimeViewE2ESplitRight,
             .requestedRuntimeViewE2ESplitDown,
             .requestedRuntimeViewE2EFocus:
            return

        case .focusDone:
            if actions.contains("set-viewed-tab") {
                if config.targetViewedTabIndex != nil || config.targetViewedTabId != nil {
                    uiTestTraceAutomationStep = .setViewedTabDone
                    advanceUITestTraceAutomationIfNeeded()
                    return
                }
                uiTestTraceAutomationStep = .requestedNewTab(originalTabId: runtimeState.viewedTabId)
                client.newTab()
                return
            }
            uiTestTraceAutomationStep = .setViewedTabDone
            advanceUITestTraceAutomationIfNeeded()

        case .requestedNewTab(let originalTabId):
            guard runtimeState.tabs.count >= 2 else { return }
            guard let currentTabId = runtimeState.viewedTabId else { return }
            let targetTabId =
                (originalTabId != nil && originalTabId != currentTabId)
                ? originalTabId
                : runtimeState.tabs.first(where: { $0.tabId != currentTabId })?.tabId
            guard let targetTabId else { return }
            uiTestTraceAutomationStep = .requestedSetViewedTab(tabId: targetTabId)
            client.setViewedTab(targetTabId)

        case .requestedSetViewedTab(let tabId):
            guard runtimeState.viewedTabId == tabId else { return }
            uiTestTraceAutomationStep = .setViewedTabDone
            advanceUITestTraceAutomationIfNeeded()

        case .setViewedTabDone:
            if actions.contains("input") {
                if actions.contains("runtime-view-e2e"),
                   !focusedRuntimePaneHasText(runtimeState) {
                    return
                }
                uiTestTraceAutomationStep = .sentInput
                sendUITestTraceInputIfNeeded()
                return
            }
            uiTestTraceAutomationStep = .done

        case .sentInput:
            if didSendUITestTraceInput {
                uiTestTraceAutomationStep = .done
            }

        case .done:
            return
        }
    }

    private func uiTestTargetViewedTabId(
        _ config: UITestLaunchConfiguration,
        _ runtimeState: RemoteRuntimeStateSnapshot
    ) -> UInt32? {
        if let targetViewedTabId = config.targetViewedTabId {
            return targetViewedTabId
        }
        if let index = config.targetViewedTabIndex,
           runtimeState.tabs.indices.contains(index)
        {
            return runtimeState.tabs[index].tabId
        }
        return nil
    }

    private func focusedRuntimePaneHasText(_ runtimeState: RemoteRuntimeStateSnapshot) -> Bool {
        let paneText = client.paneAccessibilityText(paneId: runtimeState.focusedPane)
        if paneText.contains(where: { !$0.isWhitespace }) {
            return true
        }
        return client.screen.accessibilityTextSnapshot.contains(where: { !$0.isWhitespace })
    }

    private func advanceUITestRuntimeViewE2EIfNeeded(_ runtimeState: RemoteRuntimeStateSnapshot) -> Bool {
        switch uiTestTraceAutomationStep {
        case .idle:
            if runtimeState.tabs.count < 2 {
                uiTestTraceAutomationStep = .requestedRuntimeViewE2ENewTab
                client.newTab()
                return true
            }
            guard runtimeState.tabs.indices.contains(1) else { return true }
            let targetTabId = runtimeState.tabs[1].tabId
            if runtimeState.viewedTabId != targetTabId {
                uiTestTraceAutomationStep = .requestedRuntimeViewE2ESetViewedTab(tabId: targetTabId)
                client.setViewedTab(targetTabId)
                return true
            }
            if runtimeState.visiblePanes.count < 2 {
                uiTestTraceAutomationStep = .requestedRuntimeViewE2ESplitRight
                client.newSplit(direction: "right")
                return true
            }
            if runtimeState.visiblePanes.count < 3 {
                uiTestTraceAutomationStep = .requestedRuntimeViewE2ESplitDown
                client.newSplit(direction: "down")
                return true
            }
            if let tabId = runtimeState.viewedTabId,
               let targetPane = runtimeState.visiblePanes.first(where: { !$0.focused })
            {
                uiTestTraceAutomationStep = .requestedRuntimeViewE2EFocus(paneId: targetPane.paneId)
                client.focusPane(tabId: tabId, paneId: targetPane.paneId)
                return true
            }
            uiTestTraceAutomationStep = .setViewedTabDone
            advanceUITestTraceAutomationIfNeeded()
            return true

        case .requestedRuntimeViewE2ENewTab:
            guard runtimeState.tabs.count >= 2 else { return true }
            uiTestTraceAutomationStep = .idle
            advanceUITestTraceAutomationIfNeeded()
            return true

        case .requestedRuntimeViewE2ESetViewedTab(let tabId):
            guard runtimeState.viewedTabId == tabId else { return true }
            uiTestTraceAutomationStep = .idle
            advanceUITestTraceAutomationIfNeeded()
            return true

        case .requestedRuntimeViewE2ESplitRight:
            guard runtimeState.visiblePanes.count >= 2 else { return true }
            uiTestTraceAutomationStep = .idle
            advanceUITestTraceAutomationIfNeeded()
            return true

        case .requestedRuntimeViewE2ESplitDown:
            guard runtimeState.visiblePanes.count >= 3 else { return true }
            uiTestTraceAutomationStep = .idle
            advanceUITestTraceAutomationIfNeeded()
            return true

        case .requestedRuntimeViewE2EFocus(let paneId):
            guard runtimeState.focusedPane == paneId else { return true }
            uiTestTraceAutomationStep = .setViewedTabDone
            advanceUITestTraceAutomationIfNeeded()
            return true

        default:
            return false
        }
    }

    private func requestUITestFocusPane(_ runtimeState: RemoteRuntimeStateSnapshot) {
        guard let tabId = runtimeState.viewedTabId,
              let targetPane = runtimeState.visiblePanes.first(where: { !$0.focused })
        else { return }
        uiTestTraceAutomationStep = .requestedFocus(paneId: targetPane.paneId)
        client.focusPane(tabId: tabId, paneId: targetPane.paneId)
    }

    private func sendTypedText(_ text: String) {
        guard !text.isEmpty else { return }

        if ctrlModifierState.isActive, text.count == 1, let first = text.first, first.isLetter,
           let ascii = first.uppercased().first?.asciiValue
        {
            client.sendInputBytes(Data([ascii - 64]))
            consumeModifier(.ctrl)
            return
        }

        if altModifierState.isActive || metaModifierState.isActive {
            client.sendInputBytes(Data([0x1b]))
            consumeModifier(.alt)
            consumeModifier(.meta)
        }

        if text == "\r" {
            client.sendInputBytes(Data([0x0d]))
            return
        }

        client.sendInput(text)
    }

    private func handleTerminalGesture(
        _ action: RemoteTerminalGestureAction,
        paneId: UInt64? = nil,
        viewedTabId: UInt32? = nil,
        isFocused: Bool = true
    ) {
        func focusPaneForGesture(showKeyboard: Bool) {
            if let paneId, let viewedTabId {
                if focusRuntimePaneForGesture(paneId, viewedTabId: viewedTabId) {
                    showTerminalGestureStatus("Focused pane \(paneId)")
                }
            }
            if showKeyboard, !isDisconnected, client.activeTabId != nil {
                requestKeyboardFocus()
            }
        }

        switch action {
        case .tap:
            focusPaneForGesture(showKeyboard: true)
        case .longPress:
            focusPaneForGesture(showKeyboard: true)
            showTerminalGestureStatus("Compose input")
            showingComposeInput = true
        case .twoFingerTap:
            focusPaneForGesture(showKeyboard: true)
            showTerminalGestureStatus("Compose input")
            showingComposeInput = true
        case .pageUp:
            sendSpecialKey([0x1b, 0x5b, 0x35, 0x7e])
        case .pageDown:
            sendSpecialKey([0x1b, 0x5b, 0x36, 0x7e])
        case .arrowLeft:
            sendSpecialKey([0x1b, 0x5b, 0x44])
        case .arrowRight:
            sendSpecialKey([0x1b, 0x5b, 0x43])
        case .scrollRows(let rows):
            if isFocused {
                if let paneId {
                    client.scrollTouchHistory(paneId: paneId, rows: rows)
                }
            } else {
                showTerminalGestureStatus("Focus before scroll")
                focusPaneForGesture(showKeyboard: false)
            }
        }
    }

    private func showTerminalGestureStatus(_ message: String) {
        terminalGestureStatusGeneration &+= 1
        let generation = terminalGestureStatusGeneration
        terminalGestureStatus = message
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            guard terminalGestureStatusGeneration == generation else { return }
            terminalGestureStatus = nil
        }
    }


    private func sendFunctionKey(_ index: Int) {
        let bytes: [UInt8]
        switch index {
        case 1: bytes = [0x1b, 0x4f, 0x50]
        case 2: bytes = [0x1b, 0x4f, 0x51]
        case 3: bytes = [0x1b, 0x4f, 0x52]
        case 4: bytes = [0x1b, 0x4f, 0x53]
        case 5: bytes = [0x1b, 0x5b, 0x31, 0x35, 0x7e]
        case 6: bytes = [0x1b, 0x5b, 0x31, 0x37, 0x7e]
        case 7: bytes = [0x1b, 0x5b, 0x31, 0x38, 0x7e]
        case 8: bytes = [0x1b, 0x5b, 0x31, 0x39, 0x7e]
        case 9: bytes = [0x1b, 0x5b, 0x32, 0x30, 0x7e]
        case 10: bytes = [0x1b, 0x5b, 0x32, 0x31, 0x7e]
        case 11: bytes = [0x1b, 0x5b, 0x32, 0x33, 0x7e]
        case 12: bytes = [0x1b, 0x5b, 0x32, 0x34, 0x7e]
        default: return
        }
        sendSpecialKey(bytes)
    }

    private func sendSpecialKey(_ bytes: [UInt8]) {
        var payload = Data()
        if altModifierState.isActive || metaModifierState.isActive {
            payload.append(0x1b)
            consumeModifier(.alt)
            consumeModifier(.meta)
        }
        if ctrlModifierState.isActive, bytes.count == 1, let ascii = asciiControlByte(for: bytes[0]) {
            payload.append(ascii)
            consumeModifier(.ctrl)
        } else {
            payload.append(contentsOf: bytes)
        }
        client.sendInputBytes(payload)
    }

    private func handleKeyCommand(input: String, modifiers: UIKeyModifierFlags) -> Bool {
        let terminalModifiers = modifiers.intersection([.shift, .alphaShift, .control, .alternate, .command])

        switch input {
        case UIKeyCommand.inputUpArrow:
            sendSpecialKey([0x1b, 0x5b, 0x41])
            return true
        case UIKeyCommand.inputDownArrow:
            sendSpecialKey([0x1b, 0x5b, 0x42])
            return true
        case UIKeyCommand.inputLeftArrow:
            sendSpecialKey([0x1b, 0x5b, 0x44])
            return true
        case UIKeyCommand.inputRightArrow:
            sendSpecialKey([0x1b, 0x5b, 0x43])
            return true
        case "\t":
            sendSpecialKey([0x09])
            return true
        case "\r":
            client.sendInputBytes(Data([0x0d]))
            return true
        case "\u{1b}":
            sendSpecialKey([0x1b])
            return true
        default:
            break
        }

        let hasTerminalModifiers = terminalModifiers.contains(.control) || terminalModifiers.contains(.alternate) || terminalModifiers.contains(.command)
        guard hasTerminalModifiers else {
            return false
        }

        guard !input.isEmpty else {
            return false
        }

        var payload = Data()
        if terminalModifiers.contains(.alternate) || terminalModifiers.contains(.command) {
            payload.append(0x1b)
        }

        if terminalModifiers.contains(.control), input.count == 1, let scalar = input.uppercased().unicodeScalars.first {
            let value = scalar.value
            if (0x40...0x5f).contains(value) {
                payload.append(UInt8(value - 64))
                client.sendInputBytes(payload)
                return true
            }
        }

        if input == "\r" || input == "\n" {
            payload.append(0x0d)
            client.sendInputBytes(payload)
            return true
        }

        if let encoded = input.data(using: .utf8) {
            payload.append(encoded)
            client.sendInputBytes(payload)
            return true
        }

        return false
    }

    private enum TerminalModifierKind {
        case ctrl
        case alt
        case meta
    }

    private func handleModifierEvent(_ event: TerminalAssistantModifierEvent, modifier: TerminalModifierKind) {
        switch event {
        case .pressBegan:
            setModifierState(.held, for: modifier)
            setModifierConsumed(false, for: modifier)
        case .pressEnded(let wasTap):
            let consumed = modifierConsumed(for: modifier)
            let state = modifierState(for: modifier)
            if consumed {
                if state == .held {
                    setModifierState(.inactive, for: modifier)
                }
            } else if wasTap {
                let next: TerminalModifierState = state == .latched ? .inactive : .latched
                setModifierState(next, for: modifier)
            } else if state == .held {
                setModifierState(.inactive, for: modifier)
            }
            setModifierConsumed(false, for: modifier)
        }
    }

    private func consumeModifier(_ modifier: TerminalModifierKind) {
        if modifierState(for: modifier) == .latched {
            setModifierState(.inactive, for: modifier)
        } else if modifierState(for: modifier) == .held {
            setModifierConsumed(true, for: modifier)
        }
    }

    private func resetModifierStates() {
        ctrlModifierState = .inactive
        altModifierState = .inactive
        metaModifierState = .inactive
        ctrlModifierConsumedWhileHeld = false
        altModifierConsumedWhileHeld = false
        metaModifierConsumedWhileHeld = false
    }

    private func modifierState(for modifier: TerminalModifierKind) -> TerminalModifierState {
        switch modifier {
        case .ctrl:
            return ctrlModifierState
        case .alt:
            return altModifierState
        case .meta:
            return metaModifierState
        }
    }

    private func setModifierState(_ state: TerminalModifierState, for modifier: TerminalModifierKind) {
        switch modifier {
        case .ctrl:
            ctrlModifierState = state
        case .alt:
            altModifierState = state
        case .meta:
            metaModifierState = state
        }
    }

    private func modifierConsumed(for modifier: TerminalModifierKind) -> Bool {
        switch modifier {
        case .ctrl:
            return ctrlModifierConsumedWhileHeld
        case .alt:
            return altModifierConsumedWhileHeld
        case .meta:
            return metaModifierConsumedWhileHeld
        }
    }

    private func setModifierConsumed(_ consumed: Bool, for modifier: TerminalModifierKind) {
        switch modifier {
        case .ctrl:
            ctrlModifierConsumedWhileHeld = consumed
        case .alt:
            altModifierConsumedWhileHeld = consumed
        case .meta:
            metaModifierConsumedWhileHeld = consumed
        }
    }

    private func asciiControlByte(for byte: UInt8) -> UInt8? {
        switch byte {
        case 0x40:
            return 0x00
        case 0x61...0x7a:
            return byte - 0x60
        case 0x41...0x5a:
            return byte - 0x40
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

private struct EdgeBackSwipeCapture: UIViewRepresentable {
    let onBack: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onBack: onBack)
    }

    func makeUIView(context: Context) -> UIView {
        let view = UIView(frame: .zero)
        view.backgroundColor = .clear
        let recognizer = UIScreenEdgePanGestureRecognizer(
            target: context.coordinator,
            action: #selector(Coordinator.handlePan(_:))
        )
        recognizer.edges = .left
        view.addGestureRecognizer(recognizer)

        let panRecognizer = UIPanGestureRecognizer(
            target: context.coordinator,
            action: #selector(Coordinator.handlePan(_:))
        )
        view.addGestureRecognizer(panRecognizer)
        return view
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        context.coordinator.onBack = onBack
    }

    final class Coordinator: NSObject {
        var onBack: () -> Void
        private var hasTriggered = false

        init(onBack: @escaping () -> Void) {
            self.onBack = onBack
        }

        @objc func handlePan(_ recognizer: UIPanGestureRecognizer) {
            let translation = recognizer.translation(in: recognizer.view)
            switch recognizer.state {
            case .changed:
                guard !hasTriggered, translation.x >= 64, abs(translation.x) > abs(translation.y) else { return }
                hasTriggered = true
                onBack()
            case .ended, .cancelled, .failed:
                hasTriggered = false
            default:
                break
            }
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
    @ObservedObject var store: ConnectionStore
    @ObservedObject var tailscaleBrowser: TailscalePeerBrowser

    @State private var nodeName = ""
    @State private var nodeHost = ""
    @State private var nodePort = BooDefaultRemotePortText
    @State private var tailscalePort = BooDefaultRemotePortText
    @State private var tailscaleToken = ""

    var body: some View {
        VStack(spacing: 0) {
            KineticTopBar(title: "Settings", subtitle: "Manage saved nodes and current connection state.")
            ScrollView {
                VStack(alignment: .leading, spacing: KineticSpacing.xl) {
                    VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                        KineticSectionLabel(text: "Save Node")
                        KineticInputField(placeholder: "Name", text: $nodeName, accessibilityIdentifier: "settings-node-name-input")
                        KineticInputField(placeholder: "Host", text: $nodeHost, accessibilityIdentifier: "settings-node-host-input")
                        KineticInputField(placeholder: "Port", text: $nodePort, keyboardType: .numberPad, accessibilityIdentifier: "settings-node-port-input")
                        Button("Save Node") { saveNode() }
                            .buttonStyle(KineticPrimaryButtonStyle())
                            .accessibilityIdentifier("save-node-button")
                    }

                    VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                        KineticSectionLabel(text: "Tailscale Discovery")
                        Text("Use a Tailscale API access token to list tailnet devices. This does not reuse the Tailscale app connection, and it does not verify that Boo is actually running on those devices.")
                            .font(KineticFont.caption)
                            .foregroundStyle(KineticColor.onSurfaceVariant)
                        if let statusMessage = store.tailscaleTokenStatusMessage {
                            Text(statusMessage)
                                .font(KineticFont.caption)
                                .foregroundStyle(statusMessage.contains("saved securely") ? KineticColor.primary : KineticColor.error)
                                .accessibilityIdentifier("settings-tailscale-token-status")
                        } else if store.hasTailscaleAPIToken {
                            Text("API access token saved securely in the iOS Keychain.")
                                .font(KineticFont.caption)
                                .foregroundStyle(KineticColor.primary)
                                .accessibilityIdentifier("settings-tailscale-token-status")
                        }
                        KineticInputField(placeholder: "Default Boo Port", text: $tailscalePort, keyboardType: .numberPad, accessibilityIdentifier: "settings-tailscale-port-input")
                        KineticInputField(placeholder: store.hasTailscaleAPIToken ? "Replace saved Tailscale API access token" : "Tailscale API Access Token", text: $tailscaleToken, accessibilityIdentifier: "settings-tailscale-token-input")
                        Button("Save Tailscale Settings") {
                            let port = UInt16(tailscalePort) ?? BooDefaultRemotePort
                            store.updateTailscaleDiscovery(defaultPort: port)
                            if !tailscaleToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                if store.replaceTailscaleAPIToken(tailscaleToken) {
                                    tailscaleToken = ""
                                }
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

                    VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                        KineticSectionLabel(text: "Terminal Display")
                        Toggle(isOn: Binding(
                            get: { store.terminalDisplaySettings.showConnectionHealthHUD },
                            set: { store.updateTerminalDisplay(showConnectionHealthHUD: $0) }
                        )) {
                            VStack(alignment: .leading, spacing: KineticSpacing.xs) {
                                Text("Show connection health HUD")
                                    .font(KineticFont.bodySmall)
                                    .foregroundStyle(KineticColor.onSurface)
                                Text("Display RTT, view, pane revision, and pane-state issue counters above the terminal.")
                                    .font(KineticFont.caption)
                                    .foregroundStyle(KineticColor.onSurfaceVariant)
                            }
                        }
                        .tint(KineticColor.primary)
                        .accessibilityIdentifier("settings-show-connection-health-hud-toggle")
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
            if nodePort.isEmpty {
                nodePort = BooDefaultRemotePortText
            }
        }
    }

    private func saveNode() {
        guard !nodeName.isEmpty, !nodeHost.isEmpty else { return }
        let port = UInt16(nodePort) ?? BooDefaultRemotePort
        store.addNode(SavedNode(name: nodeName, host: nodeHost, port: port))
        nodeName = ""
        nodeHost = ""
        nodePort = BooDefaultRemotePortText
    }
}

private struct RuntimePaneDivider {
    enum Axis {
        case horizontal
        case vertical
    }

    let axis: Axis
    let union: CGRect
    let center: CGPoint
    let hitWidth: CGFloat
    let hitHeight: CGFloat
    let firstPaneId: UInt64
    let secondPaneId: UInt64

    var runtimeDirection: String {
        switch axis {
        case .horizontal: return "down"
        case .vertical: return "right"
        }
    }

    func normalizedRatio(for location: CGPoint) -> Double? {
        switch axis {
        case .horizontal:
            guard union.height > 1 else { return nil }
            return Double((location.y - union.minY) / union.height)
        case .vertical:
            guard union.width > 1 else { return nil }
            return Double((location.x - union.minX) / union.width)
        }
    }

    func isSame(as other: RuntimePaneDivider) -> Bool {
        axis == other.axis
            && abs(center.x - other.center.x) < 0.5
            && abs(center.y - other.center.y) < 0.5
            && abs(hitWidth - other.hitWidth) < 0.5
            && abs(hitHeight - other.hitHeight) < 0.5
    }

    static func make(first: RemoteRuntimePaneSnapshot, second: RemoteRuntimePaneSnapshot) -> RuntimePaneDivider? {
        let left = CGRect(x: first.frame.x, y: first.frame.y, width: first.frame.width, height: first.frame.height)
        let right = CGRect(x: second.frame.x, y: second.frame.y, width: second.frame.width, height: second.frame.height)

        if let divider = verticalDivider(left: first, leftFrame: left, right: second, rightFrame: right)
            ?? verticalDivider(left: second, leftFrame: right, right: first, rightFrame: left)
        {
            return divider
        }
        if let divider = horizontalDivider(top: first, topFrame: left, bottom: second, bottomFrame: right)
            ?? horizontalDivider(top: second, topFrame: right, bottom: first, bottomFrame: left)
        {
            return divider
        }
        return nil
    }

    private static func verticalDivider(
        left: RemoteRuntimePaneSnapshot,
        leftFrame: CGRect,
        right: RemoteRuntimePaneSnapshot,
        rightFrame: CGRect
    ) -> RuntimePaneDivider? {
        let leftEdge = leftFrame.maxX
        let gap = rightFrame.minX - leftEdge
        guard (0 ... 2).contains(gap) else { return nil }
        let top = max(leftFrame.minY, rightFrame.minY)
        let bottom = min(leftFrame.maxY, rightFrame.maxY)
        guard bottom - top > 1 else { return nil }
        return RuntimePaneDivider(
            axis: .vertical,
            union: leftFrame.union(rightFrame),
            center: CGPoint(x: leftEdge + gap * 0.5, y: (top + bottom) * 0.5),
            hitWidth: 28,
            hitHeight: bottom - top,
            firstPaneId: left.paneId,
            secondPaneId: right.paneId
        )
    }

    private static func horizontalDivider(
        top: RemoteRuntimePaneSnapshot,
        topFrame: CGRect,
        bottom: RemoteRuntimePaneSnapshot,
        bottomFrame: CGRect
    ) -> RuntimePaneDivider? {
        let topEdge = topFrame.maxY
        let gap = bottomFrame.minY - topEdge
        guard (0 ... 2).contains(gap) else { return nil }
        let left = max(topFrame.minX, bottomFrame.minX)
        let rightEdge = min(topFrame.maxX, bottomFrame.maxX)
        guard rightEdge - left > 1 else { return nil }
        return RuntimePaneDivider(
            axis: .horizontal,
            union: topFrame.union(bottomFrame),
            center: CGPoint(x: (left + rightEdge) * 0.5, y: topEdge + gap * 0.5),
            hitWidth: rightEdge - left,
            hitHeight: 28,
            firstPaneId: top.paneId,
            secondPaneId: bottom.paneId
        )
    }
}
