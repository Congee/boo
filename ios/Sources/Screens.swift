import SwiftUI
import Network
import UIKit
import Foundation

private func formatConnectionTarget(host: String, port: UInt16) -> String {
    port == BooDefaultRemotePort ? host : "\(host):\(port)"
}

private func endpointDisplayTarget(_ endpoint: NWEndpoint) -> (nodeName: String, host: String, port: UInt16) {
    switch endpoint {
    case .service(let name, _, _, _):
        if let parsed = parseAdvertisedServiceTarget(name) {
            return parsed
        }
        return (name, name, BooDefaultRemotePort)
    case .hostPort(let host, let port):
        let hostString = host.debugDescription
        return (hostString, hostString, port.rawValue)
    default:
        let text = "\(endpoint)"
        return (text, text, BooDefaultRemotePort)
    }
}

private final class BonjourServiceResolver: NSObject, NetServiceDelegate {
    private var completion: ((Result<(host: String, port: UInt16), Error>) -> Void)?
    private var service: NetService?

    func resolve(endpoint: NWEndpoint, completion: @escaping (Result<(host: String, port: UInt16), Error>) -> Void) {
        guard case .service(let name, let type, let domain, _) = endpoint else {
            completion(.failure(NSError(domain: "BooBonjour", code: -1, userInfo: [NSLocalizedDescriptionKey: "Bonjour resolve requires a service endpoint"])))
            return
        }

        let service = NetService(domain: domain.isEmpty ? "local." : domain, type: type, name: name)
        service.includesPeerToPeer = true
        service.delegate = self
        self.service = service
        self.completion = completion
        service.resolve(withTimeout: 5)
    }

    func netServiceDidResolveAddress(_ sender: NetService) {
        defer {
            sender.stop()
            service = nil
        }

        if let addresses = sender.addresses {
            if let host = addresses.compactMap({ parseNumericHost(from: $0, preferredFamily: AF_INET) }).first
                ?? addresses.compactMap({ parseNumericHost(from: $0, preferredFamily: AF_INET6) }).first
            {
                completion?(.success((host: host, port: UInt16(sender.port))))
                completion = nil
                return
            }
        }

        if let hostName = sender.hostName?.trimmingCharacters(in: CharacterSet(charactersIn: ".")),
           !hostName.isEmpty {
            completion?(.success((host: hostName, port: UInt16(sender.port))))
            completion = nil
            return
        }

        completion?(.failure(NSError(domain: "BooBonjour", code: -2, userInfo: [NSLocalizedDescriptionKey: "Bonjour resolved without an address"])))
        completion = nil
    }

    func netService(_ sender: NetService, didNotResolve errorDict: [String : NSNumber]) {
        defer {
            sender.stop()
            service = nil
            completion = nil
        }
        let code = errorDict[NetService.errorCode]?.intValue ?? -1
        completion?(.failure(NSError(domain: "BooBonjour", code: code, userInfo: [NSLocalizedDescriptionKey: "Bonjour resolve failed: \(code)"])))
    }

    private func parseNumericHost(from addressData: Data, preferredFamily: Int32) -> String? {
        addressData.withUnsafeBytes { rawBuffer in
            guard let sockaddr = rawBuffer.baseAddress?.assumingMemoryBound(to: sockaddr.self) else {
                return nil
            }

            switch Int32(sockaddr.pointee.sa_family) {
            case AF_INET where preferredFamily == AF_INET:
                let addr = rawBuffer.baseAddress!.assumingMemoryBound(to: sockaddr_in.self).pointee.sin_addr
                var storage = addr
                var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
                guard inet_ntop(AF_INET, &storage, &buffer, socklen_t(INET_ADDRSTRLEN)) != nil else {
                    return nil
                }
                return String(cString: buffer)
            case AF_INET6 where preferredFamily == AF_INET6:
                let addr = rawBuffer.baseAddress!.assumingMemoryBound(to: sockaddr_in6.self).pointee.sin6_addr
                var storage = addr
                var buffer = [CChar](repeating: 0, count: Int(INET6_ADDRSTRLEN))
                guard inet_ntop(AF_INET6, &storage, &buffer, socklen_t(INET6_ADDRSTRLEN)) != nil else {
                    return nil
                }
                return String(cString: buffer)
            default:
                return nil
            }
        }
    }
}

private func parseAdvertisedServiceTarget(_ serviceName: String) -> (nodeName: String, host: String, port: UInt16)? {
    guard
        let regex = try? NSRegularExpression(pattern: #"^boo on (.+) \((\d+)\)$"#),
        let match = regex.firstMatch(in: serviceName, range: NSRange(serviceName.startIndex..., in: serviceName)),
        match.numberOfRanges == 3,
        let hostRange = Range(match.range(at: 1), in: serviceName),
        let portRange = Range(match.range(at: 2), in: serviceName),
        let port = UInt16(serviceName[portRange])
    else {
        return nil
    }

    let hostLabel = String(serviceName[hostRange]).trimmingCharacters(in: .whitespacesAndNewlines)
    guard !hostLabel.isEmpty else { return nil }
    return (hostLabel, "\(hostLabel).local", port)
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
        let host: String?
        let port: UInt16
        let endpoint: NWEndpoint?
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
            let latency: Double? = if let endpoint = target.endpoint {
                await measureBooQUICHandshakeLatency(endpoint: endpoint)
            } else if let host = target.host {
                await measureBooQUICHandshakeLatency(host: host, port: target.port)
            } else {
                nil
            }
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
    @StateObject private var browser = BonjourBrowser()
    @StateObject private var tailscaleBrowser = TailscalePeerBrowser()
    @StateObject private var store = ConnectionStore()
    @State private var selectedTab: BooTab = .terminal
    @State private var showingConnectedTerminal = false
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
                switch selectedTab {
                case .terminal:
                    ZStack {
                        ConnectScreen(
                            client: client,
                            browser: browser,
                            tailscaleBrowser: tailscaleBrowser,
                            store: store,
                            monitor: activeMonitor,
                            selectedTab: $selectedTab,
                            onPresentConnectedTerminal: {
                                showingConnectedTerminal = true
                            },
                            serverIdentityWarning: serverIdentityWarning
                        )
                        .opacity(showingConnectedTerminal ? 0 : 1)
                        .allowsHitTesting(!showingConnectedTerminal)
                        .accessibilityHidden(showingConnectedTerminal)
                        if showingConnectedTerminal {
                            TerminalTabScreen(
                                client: client,
                                monitor: activeMonitor,
                                store: store,
                                serverIdentityWarning: serverIdentityWarning,
                                onBack: {
                                    showingConnectedTerminal = false
                                }
                            )
                            .ignoresSafeArea()
                            .zIndex(1)
                        }
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
            if !(selectedTab == .terminal && showingConnectedTerminal) {
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

    private func applyUITestLaunchConfigurationIfNeeded() {
        guard !didApplyUITestLaunchConfiguration else { return }
        didApplyUITestLaunchConfiguration = true
        guard let config = UITestLaunchConfiguration.current(),
              config.autoConnect,
              let host = config.host
        else { return }

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
    @ObservedObject var browser: BonjourBrowser
    @ObservedObject var tailscaleBrowser: TailscalePeerBrowser
    @ObservedObject var store: ConnectionStore
    @ObservedObject var monitor: ConnectionMonitor
    @Binding var selectedTab: BooTab
    let onPresentConnectedTerminal: () -> Void
    let serverIdentityWarning: String?

    @StateObject private var dashboardProbeMonitor = DashboardProbeMonitor()
    @State private var host = ""
    @State private var serviceResolver: BonjourServiceResolver?
    @State private var resolvingBonjourService = false
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
            if resolvingBonjourService {
                return ("Resolving discovered host…", KineticColor.primary)
            }
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
            browser.startBrowsing()
            tailscaleBrowser.refresh(store: store)
            refreshDashboardProbes()
        }
        .onDisappear {
            browser.stopBrowsing()
        }
        .onChange(of: store.tailscaleDiscoverySettings) { _, _ in
            tailscaleBrowser.refresh(store: store)
        }
        .onChange(of: browser.daemons) { _, _ in
            refreshDashboardProbes()
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
            discoveredSection
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
        if let serverIdentityWarning {
            Text(serverIdentityWarning)
                .font(KineticFont.caption)
                .foregroundStyle(KineticColor.error)
                .padding(KineticSpacing.md)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(KineticColor.error.opacity(0.1))
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
        if let browserError = browser.lastError {
            VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                Text(browserError)
                    .font(KineticFont.caption)
                    .foregroundStyle(KineticColor.error)
                    .accessibilityIdentifier("bonjour-error-label")
                if browserError.contains("Local network access is required") {
                    Button("Open iPad Settings") {
                        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
                        UIApplication.shared.open(url)
                    }
                    .buttonStyle(KineticSecondaryButtonStyle())
                    .accessibilityIdentifier("open-local-network-settings-button")
                }
            }
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

    @ViewBuilder
    private var discoveredSection: some View {
        if !browser.daemons.isEmpty || browser.isSearching {
            VStack(alignment: .leading, spacing: KineticSpacing.sm) {
                KineticSectionLabel(text: "Discovered on Network")
                Text("Bonjour discovery on your current LAN or Wi-Fi network.")
                    .font(KineticFont.caption)
                    .foregroundStyle(KineticColor.onSurfaceVariant)
                if browser.isSearching && browser.daemons.isEmpty {
                    ProgressView()
                        .tint(KineticColor.primary)
                }
                ForEach(browser.daemons) { daemon in
                    let display = endpointDisplayTarget(daemon.endpoint)
                    KineticCardRow(
                        icon: "terminal",
                        title: daemon.title,
                        subtitle: rowSubtitle(base: daemon.subtitle, host: display.host, port: display.port, nodeName: display.nodeName),
                        trailingText: liveMetrics(host: display.host, port: display.port, nodeName: display.nodeName),
                        trailingAccessibilityIdentifier: rowMetricAccessibilityIdentifier(nodeName: display.nodeName),
                        onTap: {
                            connectToEndpoint(daemon.endpoint)
                        },
                        accessibilityIdentifier: "discovered-daemon-\(daemon.name)"
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
                let detail = tailscalePeerDetail(peer)
                KineticCardRow(
                    icon: "network",
                    title: peer.name,
                    subtitle: rowSubtitle(base: detail, host: peer.host, port: peer.port, nodeName: peer.name),
                    trailingText: liveMetrics(host: peer.host, port: peer.port, nodeName: peer.name),
                    trailingAccessibilityIdentifier: rowMetricAccessibilityIdentifier(nodeName: peer.name),
                    subtitleAccessoryText: tailscalePortStatusText(peer),
                    subtitleAccessoryColor: tailscalePortStatusColor(peer),
                    onTap: peer.online ? {
                        connectToHost(peer.host, port: peer.port, nodeName: peer.name, routeKind: .tailscale)
                    } : nil,
                    accessibilityIdentifier: "tailscale-peer-\(peer.name)"
                )
                .opacity(peer.online ? 1.0 : 0.6)
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

    private func connectToEndpoint(_ endpoint: NWEndpoint) {
        dismissConnectScreenKeyboard()
        let display = endpointDisplayTarget(endpoint)
        if shouldReuseActiveConnection(host: display.host, port: display.port, nodeName: display.nodeName) {
            DispatchQueue.main.async {
                onPresentConnectedTerminal()
            }
            return
        }
        if case .service = endpoint {
            if let config = UITestLaunchConfiguration.current(),
               let configuredHost = config.host
            {
                connectToHost(
                    configuredHost,
                    port: config.port,
                    nodeName: display.nodeName,
                    routeKind: .bonjourLAN
                )
                return
            }
            client.lastError = nil
            resolvingBonjourService = true
            let resolver = BonjourServiceResolver()
            serviceResolver = resolver
            resolver.resolve(endpoint: endpoint) { result in
                Task { @MainActor in
                    resolvingBonjourService = false
                    serviceResolver = nil
                    switch result {
                    case .success(let resolved):
                        print("[boo-ios] resolved Bonjour service \(display.nodeName) -> \(resolved.host):\(resolved.port)")
                        connectToHost(resolved.host, port: resolved.port, nodeName: display.nodeName, routeKind: .bonjourLAN)
                    case .failure(let error):
                        client.lastError = error.localizedDescription
                    }
                }
            }
            return
        }
        let historyId = store.recordConnection(
            nodeName: display.nodeName,
            host: formatConnectionTarget(host: display.host, port: display.port)
        )
        monitor.connect(
            endpoint: endpoint,
            displayHost: display.host,
            displayPort: display.port,
            routeKind: .bonjourLAN,
            displayName: display.nodeName,
            historyId: historyId
        )
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

    private func tailscalePeerDetail(_ peer: TailscalePeer) -> String {
        var parts: [String] = []
        if let os = peer.os { parts.append(os) }
        parts.append(peer.stateDescription)
        if let address = peer.address { parts.append(address) }
        parts.append("boo:\(peer.port)")
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

    private func tailscalePortStatusText(_ peer: TailscalePeer) -> String? {
        guard tailscaleBrowser.probeMetrics[peer.id] != nil else { return nil }
        return ":\(peer.port)"
    }

    private func tailscalePortStatusColor(_ peer: TailscalePeer) -> Color {
        guard let metrics = tailscaleBrowser.probeMetrics[peer.id] else {
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
                port: node.port,
                endpoint: nil
            )
        }
        let discoveredTargets = browser.daemons.map { daemon in
            let display = endpointDisplayTarget(daemon.endpoint)
            return DashboardProbeMonitor.Target(
                key: dashboardProbeKey(host: display.host, port: display.port, nodeName: display.nodeName),
                host: display.host,
                port: display.port,
                endpoint: daemon.endpoint
            )
        }
        dashboardProbeMonitor.updateTargets(savedTargets + discoveredTargets)
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

struct TerminalTabScreen: View {
    @ObservedObject var client: GSPClient
    @ObservedObject var monitor: ConnectionMonitor
    @ObservedObject var store: ConnectionStore
    let serverIdentityWarning: String?
    let onBack: () -> Void

    @State private var keyboardFocused = false
    @State private var ctrlModifierState: TerminalModifierState = .inactive
    @State private var altModifierState: TerminalModifierState = .inactive
    @State private var metaModifierState: TerminalModifierState = .inactive
    @State private var ctrlModifierConsumedWhileHeld = false
    @State private var altModifierConsumedWhileHeld = false
    @State private var metaModifierConsumedWhileHeld = false
    @State private var didApplyUITestForcedError = false
    @State private var closingHostTabId: UInt32?

    private var visibleTabs: [RemoteTabInfo] {
        client.tabs.filter { !$0.childExited }
    }

    private var tabHealth: ActiveTabHealth {
        resolveActiveTabHealth(activeTabId: client.activeTabId, tabs: client.tabs)
    }

    private var tabHealthIssue: String? {
        tabHealth.issue
    }

    var body: some View {
        ZStack(alignment: .topLeading) {
            terminalTabBody
                .background(KineticColor.surface)
                .ignoresSafeArea()
                .navigationBarBackButtonHidden(true)
                .toolbar(.hidden, for: .navigationBar)

            GeometryReader { geo in
                Color.clear
                    .frame(width: min(max(geo.size.width * 0.14, 56), 104))
                    .contentShape(Rectangle())
                    .accessibilityIdentifier("terminal-back-swipe-zone")
                    .highPriorityGesture(
                        DragGesture(minimumDistance: 20)
                            .onEnded { drag in
                                let dx = drag.translation.width
                                let dy = drag.translation.height
                                guard dx >= 64, abs(dx) > abs(dy) else { return }
                                goBack()
                            }
                    )
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)

            HStack {
                if store.terminalDisplaySettings.showFloatingBackButton {
                    floatingBackButton
                }
                Spacer()
                floatingDisconnectButton
            }
            .padding(.horizontal, KineticSpacing.md)
            .padding(.top, 14)
            .zIndex(10)

        }
        .onAppear {
            applyUITestForcedErrorIfNeeded()
            guard !isDisconnected, client.activeTabId != nil else { return }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                keyboardFocused = true
            }
        }
        .onReceive(client.$tabs) { _ in
            if finalizeHostTabCloseIfNeeded() {
                return
            }
            applyUITestForcedErrorIfNeeded()
        }
        .onChange(of: client.activeTabId) { _, activeTabId in
            if finalizeHostTabCloseIfNeeded() {
                return
            }
            if activeTabId != nil {
                keyboardFocused = false
                DispatchQueue.main.async {
                    keyboardFocused = true
                }
                applyUITestForcedErrorIfNeeded()
            }
        }
        .onChange(of: isDisconnected) { _, disconnected in
            if disconnected {
                keyboardFocused = false
            }
        }
        .onDisappear {
            keyboardFocused = false
        }
    }

    private var terminalTabBody: some View {
        VStack(spacing: 0) {
            terminalBanner
            terminalView
            if UITestLaunchConfiguration.current() != nil {
                Color.clear
                    .frame(width: 1, height: 1)
                    .accessibilityIdentifier("terminal-debug-state")
                    .accessibilityLabel(client.uiTestTabDebugSummary)
            }
        }
    }

    @ViewBuilder
    private var attachmentOverlay: some View {
        if !isDisconnected, client.activeTabId == nil {
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
    private var terminalBanner: some View {
        if let serverIdentityWarning {
            transportBanner(reason: serverIdentityWarning, color: KineticColor.error)
        } else if let tabHealthIssue {
            transportBanner(reason: tabHealthIssue, color: KineticColor.error)
        } else if let lastError = client.lastError, !lastError.isEmpty {
            transportBanner(reason: monitor.contextualErrorMessage(lastError), color: KineticColor.error)
        } else if let disconnectReason {
            transportBanner(reason: monitor.contextualErrorMessage(disconnectReason), color: KineticColor.error)
        } else if case .degraded(let reason) = monitor.transportHealth {
            transportBanner(reason: monitor.contextualErrorMessage(reason), color: KineticColor.tertiary)
        }
    }

    private var terminalView: some View {
        let bridge = terminalKeyboardBridge

        return RemoteTerminalView(screen: client.screen) { cols, rows in
            client.sendResize(cols: cols, rows: rows)
        } onGestureAction: { action in
            handleTerminalGesture(action)
        }
        .opacity(isDisconnected || client.activeTabId == nil ? 0.5 : 1.0)
        .accessibilityIdentifier("terminal-screen")
        .accessibilityLabel(client.activeTabId.map { "active-\($0)" } ?? "inactive")
        .accessibilityValue(client.screen.accessibilityTextSnapshot)
        .contentShape(Rectangle())
        .onTapGesture {
            guard !isDisconnected, client.activeTabId != nil else { return }
            keyboardFocused = true
        }
        .overlay {
            bridge
            .id("terminal-keyboard-\(client.connectionDebugGeneration)-\(client.activeTabId ?? 0)")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .contentShape(Rectangle())
        }
        .overlay {
            attachmentOverlay
        }
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

    private var terminalAccessoryState: TerminalKeyboardAccessoryState {
        TerminalKeyboardAccessoryState(
            ctrlActive: ctrlModifierState.isActive,
            altActive: altModifierState.isActive,
            metaActive: metaModifierState.isActive,
            onInsertText: sendTypedText,
            onEscape: { sendSpecialKey([0x1b]) },
            onCtrlModifierEvent: { handleModifierEvent($0, modifier: .ctrl) },
            onAltModifierEvent: { handleModifierEvent($0, modifier: .alt) },
            onMetaModifierEvent: { handleModifierEvent($0, modifier: .meta) },
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

    private var disconnectReason: String? {
        if let tabHealthIssue {
            return tabHealthIssue
        }
        if case .connectionLost(let reason) = monitor.status {
            return reason
        }
        if case .lost(let reason) = monitor.transportHealth {
            return reason
        }
        return nil
    }

    private var activeErrorMessage: String? {
        if let serverIdentityWarning { return serverIdentityWarning }
        if let tabHealthIssue { return tabHealthIssue }
        if let lastError = client.lastError, !lastError.isEmpty { return monitor.contextualErrorMessage(lastError) }
        if let disconnectReason { return monitor.contextualErrorMessage(disconnectReason) }
        return nil
    }

    private func transportBanner(reason: String, color: Color) -> some View {
        VStack(alignment: .leading, spacing: KineticSpacing.sm) {
            Text(reason)
                .font(KineticFont.caption)
                .foregroundStyle(color)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("terminal-banner-label")

            HStack(spacing: KineticSpacing.sm) {
                if let activeTabId = client.activeTabId {
                    Button("Close Tab") {
                        forgetResumeAttachment()
                        client.suppressAutomaticTabBootstrap()
                        client.clearErrorState()
                        client.destroyTab(tabId: activeTabId)
                    }
                    .buttonStyle(KineticSecondaryButtonStyle())
                    .accessibilityIdentifier("close-tab-button")
                }

                Button("New Tab") {
                    forgetResumeAttachment()
                    client.clearErrorState()
                    client.createTab()
                }
                .buttonStyle(KineticSecondaryButtonStyle())
                .accessibilityIdentifier("new-tab-button")

                Button("Disconnect") {
                    monitor.disconnect()
                    goBack()
                }
                .buttonStyle(KineticSecondaryButtonStyle())
                .accessibilityIdentifier("disconnect-tab-button")
            }
        }
        .padding(KineticSpacing.md)
        .background(color.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
    }

    private var floatingBackButton: some View {
        Button {
            goBack()
        } label: {
            Image(systemName: "chevron.left")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(KineticColor.onSurface)
                .frame(width: 38, height: 38)
                .background(
                    ZStack {
                        Circle()
                            .fill(.ultraThinMaterial)
                        Circle()
                            .fill(
                                RadialGradient(
                                    colors: [
                                        Color.gray.opacity(0.30),
                                        Color.gray.opacity(0.18),
                                        Color.clear
                                    ],
                                    center: .center,
                                    startRadius: 6,
                                    endRadius: 38
                                )
                            )
                            .scaleEffect(1.75)
                    }
                )
                .overlay(
                    Circle()
                        .stroke(Color.white.opacity(0.30), lineWidth: 0.8)
                )
                .shadow(color: .black.opacity(0.18), radius: 18, x: 0, y: 8)
        }
        .buttonStyle(.plain)
        .contentShape(Circle())
        .accessibilityIdentifier("floating-back-button")
        .accessibilityLabel("Back")
    }

    private var floatingDisconnectButton: some View {
        Button {
            closeHostTab()
        } label: {
            Image(systemName: "xmark")
                .font(.system(size: 15, weight: .bold))
                .foregroundStyle(KineticColor.onSurface)
                .frame(width: 38, height: 38)
                .background(
                    ZStack {
                        Circle()
                            .fill(.ultraThinMaterial)
                        Circle()
                            .fill(
                                RadialGradient(
                                    colors: [
                                        Color.red.opacity(0.22),
                                        Color.red.opacity(0.12),
                                        Color.clear
                                    ],
                                    center: .center,
                                    startRadius: 6,
                                    endRadius: 38
                                )
                            )
                            .scaleEffect(1.75)
                    }
                )
                .overlay(
                    Circle()
                        .stroke(Color.white.opacity(0.30), lineWidth: 0.8)
                )
                .shadow(color: .black.opacity(0.18), radius: 18, x: 0, y: 8)
        }
        .buttonStyle(.plain)
        .contentShape(Circle())
        .accessibilityIdentifier("floating-disconnect-button")
        .accessibilityLabel("Disconnect")
    }

    private func forgetResumeAttachment() {
    }

    private func goBack() {
        keyboardFocused = false
        resetModifierStates()
        DispatchQueue.main.async {
            onBack()
        }
    }

    private func closeHostTab() {
        keyboardFocused = false
        resetModifierStates()
        forgetResumeAttachment()
        client.clearErrorState()
        let activeTabId = client.activeTabId
        if let activeTabId {
            closingHostTabId = activeTabId
            client.suppressAutomaticTabBootstrap()
            client.destroyTab(tabId: activeTabId)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                forceCloseHostTabIfNeeded(expectedTabId: activeTabId)
            }
        } else {
            monitor.disconnect()
            DispatchQueue.main.async {
                onBack()
            }
        }
    }

    private func forceCloseHostTabIfNeeded(expectedTabId: UInt32) {
        guard closingHostTabId == expectedTabId else { return }
        closingHostTabId = nil
        monitor.disconnect()
        DispatchQueue.main.async {
            onBack()
        }
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

    @discardableResult
    private func finalizeHostTabCloseIfNeeded() -> Bool {
        guard let closingHostTabId else { return false }
        let tabStillVisible = visibleTabs.contains(where: { $0.id == closingHostTabId })
        let tabStillActive = client.activeTabId == closingHostTabId
        guard !tabStillVisible, !tabStillActive else { return false }
        self.closingHostTabId = nil
        monitor.disconnect()
        DispatchQueue.main.async {
            onBack()
        }
        return true
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
        case .scrollLines(let lines):
            client.sendMouseWheelLines(y: Double(lines))
        }
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
    @State private var nodePort = BooDefaultRemotePortText
    @State private var tailscalePort = BooDefaultRemotePortText
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
                            get: { store.terminalDisplaySettings.showFloatingBackButton },
                            set: { store.updateTerminalDisplay(showFloatingBackButton: $0) }
                        )) {
                            VStack(alignment: .leading, spacing: KineticSpacing.xs) {
                                Text("Show floating Back button")
                                    .font(KineticFont.bodySmall)
                                    .foregroundStyle(KineticColor.onSurface)
                                Text("Overlay a compact Back button over the terminal. Turn this off to rely on the native back gesture only.")
                                    .font(KineticFont.caption)
                                    .foregroundStyle(KineticColor.onSurfaceVariant)
                            }
                        }
                        .tint(KineticColor.primary)
                        .accessibilityIdentifier("settings-show-floating-back-button-toggle")
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
