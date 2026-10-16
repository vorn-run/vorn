import SwiftUI

/// Approach A: vornui draws every screen; this shell hands it a layer, touches and the keyboard.
@main
struct DrawnApp: App {
    var body: some Scene {
        WindowGroup {
            Paired { core in
                Metal(core: core, terminalOnly: false)
                    .ignoresSafeArea()
                    .onAppear {
                        Report.start(core, label: "drawn")
                        if Bench.wanted {
                            Bench(core: core, label: "drawn", drawnList: true) { vp_open_session(core.handle, $0) }.start()
                        }
                    }
            }
            .preferredColorScheme(.dark)
            .statusBarHidden(false)
        }
    }
}
