import Foundation

struct RemoteTabInfo: Identifiable {
    let id: UInt32
    let name: String
    let title: String
    let pwd: String
    let attached: Bool
    let childExited: Bool
}
