import Foundation

private struct PaneStateOrderingModel {
    private var paneRevisions: [UInt64: UInt64] = [:]
    private var runtimeRevisions: [UInt64: UInt64] = [:]
    private var fullStateSeen: Set<UInt64> = []
    private var refreshRequested: [UInt64: UInt64] = [:]

    mutating func acceptFull(_ update: DecodedPaneUpdate) -> Bool {
        guard isNotOlder(update) else { return false }
        accept(update, fullState: true)
        return true
    }

    mutating func acceptDelta(_ update: DecodedPaneUpdate, deltaApplies: Bool) -> DeltaResult {
        guard isNewer(update) else { return .ignoredStaleOrDuplicate }
        guard fullStateSeen.contains(update.paneId) else {
            return .rejectedMissingBase(requestedRefresh: requestRefresh(update))
        }
        guard deltaApplies else {
            return .rejectedInvalidDelta(requestedRefresh: requestRefresh(update))
        }
        accept(update, fullState: false)
        return .accepted
    }

    mutating func mirrorFocusedLegacyFullState(paneId: UInt64, runtimeRevision: UInt64) {
        fullStateSeen.insert(paneId)
        runtimeRevisions[paneId] = runtimeRevision
    }

    func revisionSummary(_ paneId: UInt64) -> (pane: UInt64, runtime: UInt64, hasBase: Bool) {
        (paneRevisions[paneId] ?? 0, runtimeRevisions[paneId] ?? 0, fullStateSeen.contains(paneId))
    }

    private func isNotOlder(_ update: DecodedPaneUpdate) -> Bool {
        let lastPane = paneRevisions[update.paneId] ?? 0
        let lastRuntime = runtimeRevisions[update.paneId] ?? 0
        return update.paneRevision > lastPane
            || (update.paneRevision == lastPane && update.runtimeRevision >= lastRuntime)
    }

    private func isNewer(_ update: DecodedPaneUpdate) -> Bool {
        let lastPane = paneRevisions[update.paneId] ?? 0
        let lastRuntime = runtimeRevisions[update.paneId] ?? 0
        return update.paneRevision > lastPane
            || (update.paneRevision == lastPane && update.runtimeRevision > lastRuntime)
    }

    private mutating func accept(_ update: DecodedPaneUpdate, fullState: Bool) {
        paneRevisions[update.paneId] = update.paneRevision
        runtimeRevisions[update.paneId] = update.runtimeRevision
        if fullState {
            fullStateSeen.insert(update.paneId)
        }
        refreshRequested.removeValue(forKey: update.paneId)
    }

    private mutating func requestRefresh(_ update: DecodedPaneUpdate) -> Bool {
        if refreshRequested[update.paneId] == update.paneRevision { return false }
        refreshRequested[update.paneId] = update.paneRevision
        return true
    }
}

private enum DeltaResult: Equatable {
    case accepted
    case ignoredStaleOrDuplicate
    case rejectedMissingBase(requestedRefresh: Bool)
    case rejectedInvalidDelta(requestedRefresh: Bool)
}

private func update(pane: UInt64 = 7, paneRevision: UInt64, runtimeRevision: UInt64) -> DecodedPaneUpdate {
    DecodedPaneUpdate(tabId: 1, paneId: pane, paneRevision: paneRevision, runtimeRevision: runtimeRevision)
}

private func expect(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        fputs("pane state ordering self-test failed: \(message)\n", stderr)
        exit(1)
    }
}

func runPaneStateOrderingSelfTest() {
    var outOfOrder = PaneStateOrderingModel()
    expect(outOfOrder.acceptFull(update(paneRevision: 3, runtimeRevision: 3)), "new full state should be accepted")
    expect(!outOfOrder.acceptFull(update(paneRevision: 2, runtimeRevision: 4)), "older full state should be rejected even with newer runtime revision")
    expect(outOfOrder.acceptDelta(update(paneRevision: 4, runtimeRevision: 4), deltaApplies: true) == .accepted, "new delta after a base should be accepted")

    var missingBase = PaneStateOrderingModel()
    expect(missingBase.acceptDelta(update(paneRevision: 1, runtimeRevision: 1), deltaApplies: true) == .rejectedMissingBase(requestedRefresh: true), "delta without base should request a full-state refresh")
    expect(missingBase.acceptDelta(update(paneRevision: 1, runtimeRevision: 1), deltaApplies: true) == .rejectedMissingBase(requestedRefresh: false), "same missing-base delta should not request duplicate refresh")

    var focusedMirror = PaneStateOrderingModel()
    expect(focusedMirror.acceptFull(update(paneRevision: 5, runtimeRevision: 10)), "pane-specific full state should be accepted")
    focusedMirror.mirrorFocusedLegacyFullState(paneId: 7, runtimeRevision: 9)
    expect(focusedMirror.revisionSummary(7).pane == 5, "legacy focused mirror must not lower pane revision")
    expect(focusedMirror.revisionSummary(7).runtime == 9, "legacy focused mirror may record focused runtime revision without rewriting pane revision")
    expect(!focusedMirror.acceptFull(update(paneRevision: 5, runtimeRevision: 8)), "older legacy/pane full state should not replace newer pane-specific update")

    var focusChange = PaneStateOrderingModel()
    expect(focusChange.acceptFull(update(pane: 1, paneRevision: 1, runtimeRevision: 1)), "focused pane base should be accepted")
    focusChange.mirrorFocusedLegacyFullState(paneId: 2, runtimeRevision: 2)
    expect(focusChange.acceptDelta(update(pane: 1, paneRevision: 2, runtimeRevision: 3), deltaApplies: true) == .accepted, "old focused pane input delta should still apply to its own pane after focus changes")
    expect(focusChange.acceptDelta(update(pane: 2, paneRevision: 1, runtimeRevision: 3), deltaApplies: false) == .rejectedInvalidDelta(requestedRefresh: true), "invalid delta on new focused pane should request full-state refresh")
}
