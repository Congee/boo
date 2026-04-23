import Foundation

struct RemoteTabInfo: Identifiable {
    let id: UInt32
    let name: String
    let title: String
    let pwd: String
    let active: Bool
    let childExited: Bool
}
