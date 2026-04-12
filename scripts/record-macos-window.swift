#!/usr/bin/env swift

import Foundation
import AppKit
import AVFoundation
import CoreMedia
import ScreenCaptureKit
import Darwin

@available(macOS 13.0, *)
final class Recorder: NSObject, SCStreamOutput {
    private let writer: AVAssetWriter
    private let writerInput: AVAssetWriterInput
    private let adaptor: AVAssetWriterInputPixelBufferAdaptor
    private var didStartSession = false

    init(outputURL: URL, width: Int, height: Int) throws {
        try? FileManager.default.removeItem(at: outputURL)

        writer = try AVAssetWriter(outputURL: outputURL, fileType: .mp4)
        writerInput = AVAssetWriterInput(
            mediaType: .video,
            outputSettings: [
                AVVideoCodecKey: AVVideoCodecType.h264,
                AVVideoWidthKey: width,
                AVVideoHeightKey: height,
            ]
        )
        writerInput.expectsMediaDataInRealTime = true

        adaptor = AVAssetWriterInputPixelBufferAdaptor(
            assetWriterInput: writerInput,
            sourcePixelBufferAttributes: [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
                kCVPixelBufferWidthKey as String: width,
                kCVPixelBufferHeightKey as String: height,
            ]
        )

        guard writer.canAdd(writerInput) else {
            throw NSError(domain: "record-macos-window", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "cannot add writer input"
            ])
        }
        writer.add(writerInput)
        guard writer.startWriting() else {
            throw writer.error ?? NSError(domain: "record-macos-window", code: 2)
        }
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of outputType: SCStreamOutputType) {
        guard outputType == .screen else { return }
        guard CMSampleBufferIsValid(sampleBuffer) else { return }
        guard let imageBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        guard writerInput.isReadyForMoreMediaData else { return }

        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        if !didStartSession {
            writer.startSession(atSourceTime: pts)
            didStartSession = true
        }
        adaptor.append(imageBuffer, withPresentationTime: pts)
    }

    func finish() async throws {
        writerInput.markAsFinished()
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            writer.finishWriting {
                if let error = self.writer.error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
        }
    }
}

@available(macOS 13.0, *)
func stopCaptureIgnoringAlreadyStopped(_ stream: SCStream) async throws {
    do {
        try await stream.stopCapture()
    } catch let error as NSError {
        if error.domain == SCStreamErrorDomain && error.code == -3808 {
            return
        }
        throw error
    }
}

@available(macOS 13.0, *)
func run() async throws {
    guard CommandLine.arguments.count >= 3 else {
        fputs("usage: record-macos-window.swift <owner-name> <output.mp4> [seconds|--until-exit] [window-name-substring]\n", stderr)
        exit(2)
    }

    let ownerName = CommandLine.arguments[1]
    let outputURL = URL(fileURLWithPath: CommandLine.arguments[2])
    let modeArg = CommandLine.arguments.count >= 4 ? CommandLine.arguments[3] : "--until-exit"
    let titleFilter = CommandLine.arguments.count >= 5 ? CommandLine.arguments[4] : ""

    enum StopMode {
        case duration(Double)
        case untilExit
    }

    let stopMode: StopMode
    if modeArg == "--until-exit" {
        stopMode = .untilExit
    } else if let seconds = Double(modeArg), seconds > 0 {
        stopMode = .duration(seconds)
    } else {
        fputs("record-macos-window.swift: invalid duration or mode '\(modeArg)'\n", stderr)
        exit(2)
    }

    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
    guard let window = content.windows
        .filter({ $0.owningApplication?.applicationName == ownerName })
        .filter({ titleFilter.isEmpty || ($0.title?.localizedCaseInsensitiveContains(titleFilter) ?? false) })
        .max(by: { ($0.frame.width * $0.frame.height) < ($1.frame.width * $1.frame.height) }) else {
        throw NSError(domain: "record-macos-window", code: 3, userInfo: [
            NSLocalizedDescriptionKey: "no matching window found"
        ])
    }

    let width = Int(window.frame.width)
    let height = Int(window.frame.height)
    guard let pid = window.owningApplication?.processID else {
        throw NSError(domain: "record-macos-window", code: 4, userInfo: [
            NSLocalizedDescriptionKey: "matching window has no owning process"
        ])
    }
    let filter = SCContentFilter(desktopIndependentWindow: window)
    let config = SCStreamConfiguration()
    config.width = width
    config.height = height
    config.minimumFrameInterval = CMTime(value: 1, timescale: 30)
    config.queueDepth = 8

    let recorder = try Recorder(outputURL: outputURL, width: width, height: height)
    let stream = SCStream(filter: filter, configuration: config, delegate: nil)
    let queue = DispatchQueue(label: "record-macos-window")
    try stream.addStreamOutput(recorder, type: .screen, sampleHandlerQueue: queue)
    try await stream.startCapture()

    switch stopMode {
    case .duration(let seconds):
        try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
    case .untilExit:
        while kill(pid_t(pid), 0) == 0 {
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }

    try await stopCaptureIgnoringAlreadyStopped(stream)
    try await recorder.finish()
}

if #available(macOS 13.0, *) {
    _ = NSApplication.shared
    Task {
        do {
            try await run()
            exit(0)
        } catch {
            fputs("record-macos-window.swift: \(error)\n", stderr)
            exit(1)
        }
    }
    dispatchMain()
} else {
    fputs("record-macos-window.swift requires macOS 13+\n", stderr)
    exit(1)
}
