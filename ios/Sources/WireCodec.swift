import Foundation

struct DecodedWireTabInfo: Equatable {
    let id: UInt32
    let name: String
    let title: String
    let pwd: String
    let attached: Bool
    let childExited: Bool
}

struct DecodedWireCell: Equatable {
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

struct DecodedWireScreenState: Equatable {
    var rows: UInt16
    var cols: UInt16
    var cells: [DecodedWireCell]
    var cursorX: UInt16
    var cursorY: UInt16
    var cursorVisible: Bool
    var cursorBlinking: Bool
    var cursorStyle: Int32
}

enum WireCodec {
    private static let cellEncodedLen = 12
    private static let remoteFullStateHeaderLen = 14
    private static let remoteDeltaHeaderLen = 13

    static func decodeTabList(_ data: Data) -> [DecodedWireTabInfo] {
        guard data.count >= 4 else { return [] }
        let count = data.withUnsafeBytes {
            Int(UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self)))
        }
        var offset = 4
        var items: [DecodedWireTabInfo] = []
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
                DecodedWireTabInfo(
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

    static func decodeFullState(_ data: Data) -> DecodedWireScreenState? {
        guard data.count >= remoteFullStateHeaderLen else { return nil }
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
        let cursorBlinking = data[9] != 0
        let cursorStyle = data.withUnsafeBytes {
            Int32(littleEndian: $0.loadUnaligned(fromByteOffset: 10, as: Int32.self))
        }
        let cellCount = Int(rows) * Int(cols)
        let expected = remoteFullStateHeaderLen + cellCount * cellEncodedLen
        guard data.count >= expected else { return nil }
        var cells = [DecodedWireCell](repeating: DecodedWireCell(), count: cellCount)
        data.withUnsafeBytes { buf in
            for i in 0..<cellCount {
                let base = remoteFullStateHeaderLen + (i * cellEncodedLen)
                cells[i] = DecodedWireCell(
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
        return DecodedWireScreenState(
            rows: rows,
            cols: cols,
            cells: cells,
            cursorX: cursorX,
            cursorY: cursorY,
            cursorVisible: cursorVisible,
            cursorBlinking: cursorBlinking,
            cursorStyle: cursorStyle
        )
    }

    static func applyDelta(_ data: Data, to state: inout DecodedWireScreenState) -> Bool {
        guard data.count >= remoteDeltaHeaderLen else { return false }
        let numRows = data.withUnsafeBytes {
            UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self))
        }
        state.cursorX = data.withUnsafeBytes {
            UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 2, as: UInt16.self))
        }
        state.cursorY = data.withUnsafeBytes {
            UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 4, as: UInt16.self))
        }
        state.cursorVisible = data[6] != 0
        state.cursorBlinking = data[7] != 0
        let flags = data[8]
        state.cursorStyle = data.withUnsafeBytes {
            Int32(littleEndian: $0.loadUnaligned(fromByteOffset: 9, as: Int32.self))
        }
        var offset = remoteDeltaHeaderLen
        if (flags & 0x01) != 0 {
            guard offset + 2 <= data.count else { return false }
            let scrollRows = data.withUnsafeBytes {
                Int(Int16(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: Int16.self)))
            }
            offset += 2
            applyScrollRows(scrollRows, to: &state)
        }
        for _ in 0..<numRows {
            guard offset + 6 <= data.count else { return false }
            let rowIndex = data.withUnsafeBytes {
                Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt16.self)))
            }
            let startCol = data.withUnsafeBytes {
                Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: offset + 2, as: UInt16.self)))
            }
            let numCols = data.withUnsafeBytes {
                Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: offset + 4, as: UInt16.self)))
            }
            offset += 6
            let rowBytes = numCols * cellEncodedLen
            guard offset + rowBytes <= data.count else { return false }
            guard rowIndex < Int(state.rows), startCol < Int(state.cols), startCol + numCols <= Int(state.cols) else {
                offset += rowBytes
                continue
            }
            let dstStart = rowIndex * Int(state.cols) + startCol
            data.withUnsafeBytes { buf in
                for c in 0..<numCols {
                    let base = offset + (c * cellEncodedLen)
                    state.cells[dstStart + c] = DecodedWireCell(
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
            offset += rowBytes
        }
        return true
    }

    private static func applyScrollRows(_ scrollRows: Int, to state: inout DecodedWireScreenState) {
        guard scrollRows != 0 else { return }
        let rows = Int(state.rows)
        let cols = Int(state.cols)
        guard rows > 0, cols > 0 else { return }

        var shifted = [DecodedWireCell](repeating: DecodedWireCell(), count: state.cells.count)
        for row in 0..<rows {
            let sourceRow = row + scrollRows
            guard sourceRow >= 0, sourceRow < rows else { continue }
            let dstStart = row * cols
            let srcStart = sourceRow * cols
            shifted[dstStart..<(dstStart + cols)] = state.cells[srcStart..<(srcStart + cols)]
        }
        state.cells = shifted
    }

    static func screenText(from state: DecodedWireScreenState) -> String {
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
