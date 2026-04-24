import Foundation

@main
struct TraceRenderApplySelfTestMain {
    static func main() {
        var records: [BooTraceRecord] = []
        BooTrace.installRecorder { record in
            records.append(record)
        }
        defer {
            BooTrace.installRecorder(nil)
        }

        var tracker = BooRenderTraceTracker()

        tracker.beginInput(BooTraceFields(
            interactionId: tracker.nextInteractionId(),
            viewId: 9,
            tabId: 42,
            paneId: 7,
            action: "input",
            route: "remote",
            runtimeRevision: 3,
            viewRevision: 4,
            paneRevision: 0,
            elapsedMs: 0
        ))
        tracker.completeRenderApply(
            fields: BooTraceFields(
                viewId: 9,
                tabId: 42,
                paneId: 7,
                action: "render_apply",
                route: "remote",
                runtimeRevision: 5,
                viewRevision: 6,
                paneRevision: 11,
                elapsedMs: 0
            ),
            tabId: 42
        )
        assertRenderApply(
            records,
            sourceEvent: .remoteInput,
            interactionId: 1,
            tabId: 42,
            paneId: 7,
            message: "input render apply"
        )

        let beforeMismatchedRuntimeAction = records.count
        tracker.beginRuntimeAction(.remoteSetViewedTab, BooTraceFields(
            interactionId: tracker.nextInteractionId(),
            viewId: 9,
            tabId: 42,
            paneId: 0,
            action: "set_viewed_tab",
            route: "remote",
            runtimeRevision: 6,
            viewRevision: 6,
            paneRevision: 0,
            elapsedMs: 0
        ))
        tracker.completeRenderApply(
            fields: BooTraceFields(
                viewId: 9,
                tabId: 99,
                paneId: 8,
                action: "render_apply",
                route: "remote",
                runtimeRevision: 7,
                viewRevision: 7,
                paneRevision: 0,
                elapsedMs: 0
            ),
            tabId: 99
        )
        assertNoRenderApply(
            records,
            since: beforeMismatchedRuntimeAction,
            sourceEvent: .remoteSetViewedTab,
            message: "mismatched tab should not end runtime action trace"
        )
        tracker.completeRenderApply(
            fields: BooTraceFields(
                viewId: 9,
                tabId: 42,
                paneId: 7,
                action: "render_apply",
                route: "remote",
                runtimeRevision: 8,
                viewRevision: 8,
                paneRevision: 0,
                elapsedMs: 0
            ),
            tabId: 42
        )
        assertRenderApply(
            records,
            sourceEvent: .remoteSetViewedTab,
            interactionId: 2,
            tabId: 42,
            paneId: 7,
            message: "set viewed tab render apply"
        )

        tracker.beginFocusPane(BooTraceFields(
            interactionId: tracker.nextInteractionId(),
            viewId: 9,
            tabId: 42,
            paneId: 8,
            action: "focus_pane",
            route: "remote",
            runtimeRevision: 8,
            viewRevision: 8,
            paneRevision: 0,
            elapsedMs: 0
        ))
        tracker.completeRenderApply(
            fields: BooTraceFields(
                viewId: 9,
                tabId: 42,
                paneId: 8,
                action: "render_apply",
                route: "remote",
                runtimeRevision: 9,
                viewRevision: 9,
                paneRevision: 12,
                elapsedMs: 0
            ),
            tabId: 42
        )
        assertRenderApply(
            records,
            sourceEvent: .remoteFocusPane,
            interactionId: 3,
            tabId: 42,
            paneId: 8,
            message: "focus pane render apply"
        )

        print("trace render-apply self-test passed")
    }

    private static func assertRenderApply(
        _ records: [BooTraceRecord],
        sourceEvent: BooTraceEvent,
        interactionId: UInt64,
        tabId: UInt32,
        paneId: UInt64,
        message: String
    ) {
        let matched = records.contains { record in
            record.event == .remoteRenderApply
                && record.phase == "end"
                && record.sourceEvent == sourceEvent
                && record.fields.interactionId == interactionId
                && record.fields.tabId == tabId
                && record.fields.paneId == paneId
                && record.fields.action == "render_apply"
                && record.fields.elapsedMs >= 0
        }
        if !matched {
            fail("missing \(message); records=\(describe(records))")
        }
    }

    private static func assertNoRenderApply(
        _ records: [BooTraceRecord],
        since startIndex: Int,
        sourceEvent: BooTraceEvent,
        message: String
    ) {
        let tail = records.dropFirst(startIndex)
        let matched = tail.contains { record in
            record.event == .remoteRenderApply
                && record.phase == "end"
                && record.sourceEvent == sourceEvent
        }
        if matched {
            fail("unexpected \(message); records=\(describe(Array(tail)))")
        }
    }

    private static func describe(_ records: [BooTraceRecord]) -> String {
        records
            .map { record in
                "event=\(record.event.rawValue) phase=\(record.phase) source=\(record.sourceEvent?.rawValue ?? "nil") \(record.fields.summary)"
            }
            .joined(separator: " | ")
    }

    private static func fail(_ message: String) -> Never {
        fputs("assertion failed: \(message)\n", stderr)
        exit(1)
    }
}
