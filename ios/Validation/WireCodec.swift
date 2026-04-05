import Foundation

struct ValidationSessionInfo: Equatable {
    let id: UInt32
    let name: String
    let title: String
    let pwd: String
    let attached: Bool
    let childExited: Bool
}

struct ValidationWireCell: Equatable {
    var codepoint: UInt32 = 0
    var fg_r: UInt8 = 0
    var fg_g: UInt8 = 0
    var fg_b: UInt8 = 0
    var bg_r: UInt8 = 0
    var bg_g: UInt8 = 0
    var bg_b: UInt8 = 0
    var styleFlags: UInt8 = 0
    var wide: UInt8 = 0
}

struct ValidationScreenState: Equatable {
    var rows: UInt16
    var cols: UInt16
    var cells: [ValidationWireCell]
    var cursorX: UInt16
    var cursorY: UInt16
    var cursorVisible: Bool
}

enum WireCodec {
    static func decodeSessionList(_ data: Data) -> [ValidationSessionInfo] {
        guard data.count >= 4 else { return [] }
        let count = data.withUnsafeBytes {
            Int(UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self)))
        }
        var offset = 4
        var items: [ValidationSessionInfo] = []
        func readString() -> String {
            guard offset + 2 <= data.count else { return "" }
            let len = data.withUnsafeBytes {
                Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt16.self)))
            }
            offset += 2
            guard offset + len <= data.count else { return "" }
            let value = String(data: data[offset..<(offset + len)], encoding: .utf8) ?? ""
            offset += len
            return value
        }
        for _ in 0..<count {
            guard offset + 4 <= data.count else { break }
            let id = data.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
            }
            offset += 4
            let name = readString()
            let title = readString()
            let pwd = readString()
            guard offset < data.count else { break }
            let flags = data[offset]
            offset += 1
            items.append(
                ValidationSessionInfo(
                    id: id,
                    name: name,
                    title: title,
                    pwd: pwd,
                    attached: (flags & 0x01) != 0,
                    childExited: (flags & 0x02) != 0
                )
            )
        }
        return items
    }

    static func decodeFullState(_ data: Data) -> ValidationScreenState? {
        guard data.count >= 12 else { return nil }
        let rows = data.withUnsafeBytes {
            UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self))
        }
        let cols = data.withUnsafeBytes {
            UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 2, as: UInt16.self))
        }
        let cursorX = data.withUnsafeBytes {
            UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 4, as: UInt16.self))
        }
        let cursorY = data.withUnsafeBytes {
            UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 6, as: UInt16.self))
        }
        let cursorVisible = data[8] != 0
        let cellCount = Int(rows) * Int(cols)
        let expected = 12 + cellCount * 12
        guard data.count >= expected else { return nil }
        var cells = [ValidationWireCell](repeating: ValidationWireCell(), count: cellCount)
        data.withUnsafeBytes { buf in
            for i in 0..<cellCount {
                let base = 12 + (i * 12)
                cells[i] = ValidationWireCell(
                    codepoint: UInt32(littleEndian: buf.loadUnaligned(fromByteOffset: base, as: UInt32.self)),
                    fg_r: buf[base + 4],
                    fg_g: buf[base + 5],
                    fg_b: buf[base + 6],
                    bg_r: buf[base + 7],
                    bg_g: buf[base + 8],
                    bg_b: buf[base + 9],
                    styleFlags: buf[base + 10],
                    wide: buf[base + 11]
                )
            }
        }
        return ValidationScreenState(
            rows: rows,
            cols: cols,
            cells: cells,
            cursorX: cursorX,
            cursorY: cursorY,
            cursorVisible: cursorVisible
        )
    }

    static func screenText(from state: ValidationScreenState) -> String {
        var text = ""
        for row in 0..<Int(state.rows) {
            for col in 0..<Int(state.cols) {
                let index = row * Int(state.cols) + col
                let codepoint = state.cells[index].codepoint
                if codepoint == 0 {
                    text.append(" ")
                } else if let scalar = UnicodeScalar(codepoint) {
                    text.append(Character(scalar))
                }
            }
            if row + 1 < Int(state.rows) {
                text.append("\n")
            }
        }
        return text
    }
}
