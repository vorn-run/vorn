import AVFoundation
import VornCore

/// The notification tones: a decaying sine wave per reason (playNotificationSound).
@MainActor
public enum NotificationTone {
    private static var engine: AVAudioEngine?
    private static var player: AVAudioPlayerNode?

    public static func play(_ reason: NotificationReason, volume: Double) {
        let (frequency, duration) = reason.tone
        guard let buffer = buffer(frequency: frequency, duration: duration, volume: volume) else { return }
        guard let player = ready(format: buffer.format) else { return }
        player.scheduleBuffer(buffer, at: nil, options: .interrupts)
        player.play()
    }

    /// The samples: `volume` falling exponentially to 0.001 over `duration`.
    nonisolated static func samples(frequency: Double, duration: Double, volume: Double, rate: Double) -> [Float] {
        let count = Int(duration * rate)
        let start = max(0.001, volume)
        let k = log(0.001 / start) / duration
        return (0..<count).map { i in
            let t = Double(i) / rate
            return Float(start * exp(k * t) * sin(2 * .pi * frequency * t))
        }
    }

    private static func buffer(frequency: Double, duration: Double, volume: Double) -> AVAudioPCMBuffer? {
        let rate = 44_100.0
        guard let format = AVAudioFormat(standardFormatWithSampleRate: rate, channels: 1) else { return nil }
        let data = samples(frequency: frequency, duration: duration, volume: volume, rate: rate)
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(data.count)),
              let channel = buffer.floatChannelData?[0] else { return nil }
        buffer.frameLength = AVAudioFrameCount(data.count)
        data.withUnsafeBufferPointer { channel.update(from: $0.baseAddress!, count: data.count) }
        return buffer
    }

    private static func ready(format: AVAudioFormat) -> AVAudioPlayerNode? {
        if let engine, let player, engine.isRunning { return player }
        let engine = AVAudioEngine()
        let player = AVAudioPlayerNode()
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: format)
        do { try engine.start() } catch { return nil }
        self.engine = engine
        self.player = player
        return player
    }
}
