import Foundation

extension GSPClient {
    var tabs: [RemoteTabInfo] {
        get { sessions }
        set { sessions = newValue }
    }

    var attachedTabId: UInt32? {
        get { attachedSessionId }
        set { attachedSessionId = newValue }
    }

    var pendingAttachedTabId: UInt32? {
        get { pendingAttachedSessionId }
        set { pendingAttachedSessionId = newValue }
    }

    func listTabs() {
        listSessions()
    }

    func createTab(cols: UInt16 = 120, rows: UInt16 = 36) {
        createSession(cols: cols, rows: rows)
    }

    func destroyTab(tabId: UInt32) {
        destroySession(sessionId: tabId)
    }

    func attach(tabId: UInt32) {
        attach(sessionId: tabId)
    }

    func configurePreferredHostTab(tabId: UInt32?) {
        configurePreferredHostSession(sessionId: tabId)
    }
}
