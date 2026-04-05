import Foundation

func assertEqual<T: Equatable>(_ actual: T, _ expected: T, _ message: String) {
    if actual != expected {
        fputs("assertion failed: \(message)\nactual: \(actual)\nexpected: \(expected)\n", stderr)
        exit(1)
    }
}

func makeSessionListPayload() -> Data {
    var data = Data()
    data.append(contentsOf: UInt32(1).littleEndianBytes)
    data.append(contentsOf: UInt32(7).littleEndianBytes)
    data.append(contentsOf: UInt16(5).littleEndianBytes)
    data.append("Tab 1".data(using: .utf8)!)
    data.append(contentsOf: UInt16(5).littleEndianBytes)
    data.append("shell".data(using: .utf8)!)
    data.append(contentsOf: UInt16(4).littleEndianBytes)
    data.append("/tmp".data(using: .utf8)!)
    data.append(0x01)
    return data
}

func makeFullStatePayload() -> Data {
    var data = Data()
    data.append(contentsOf: UInt16(1).littleEndianBytes)
    data.append(contentsOf: UInt16(2).littleEndianBytes)
    data.append(contentsOf: UInt16(1).littleEndianBytes)
    data.append(contentsOf: UInt16(0).littleEndianBytes)
    data.append(1)
    data.append(contentsOf: [0, 0, 0])

    data.append(contentsOf: UInt32(Character("A").unicodeScalars.first!.value).littleEndianBytes)
    data.append(contentsOf: [1, 2, 3, 4, 5, 6, 0x21, 0])

    data.append(contentsOf: UInt32(Character("好").unicodeScalars.first!.value).littleEndianBytes)
    data.append(contentsOf: [7, 8, 9, 10, 11, 12, 0x22, 1])
    return data
}

func makeDeltaPayload() -> Data {
    var data = Data()
    data.append(contentsOf: UInt16(1).littleEndianBytes)
    data.append(contentsOf: UInt16(0).littleEndianBytes)
    data.append(contentsOf: UInt16(0).littleEndianBytes)
    data.append(1)
    data.append(contentsOf: [0])
    data.append(contentsOf: UInt16(0).littleEndianBytes)
    data.append(contentsOf: UInt16(2).littleEndianBytes)

    data.append(contentsOf: UInt32(Character("B").unicodeScalars.first!.value).littleEndianBytes)
    data.append(contentsOf: [13, 14, 15, 16, 17, 18, 0x11, 0])

    data.append(contentsOf: UInt32(Character("C").unicodeScalars.first!.value).littleEndianBytes)
    data.append(contentsOf: [19, 20, 21, 22, 23, 24, 0x12, 0])
    return data
}

extension FixedWidthInteger {
    var littleEndianBytes: [UInt8] {
        withUnsafeBytes(of: self.littleEndian, Array.init)
    }
}
