//! The Apple Pencil's own gestures: a double tap on its side is Escape (it ends the tool in hand,
//! the way a tablet CAD app's pencil switches tools back), and a squeeze (Pencil Pro) opens the
//! context menu at the pencil's tip. Touches are sorted in `solidcraft_ui_egui::pencil`.
//!
//! UIKit reports these through a delegate object, which has no safe Rust API, so (like `picker`)
//! this module allows `unsafe`.
#![allow(unsafe_code)]
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{ClassType as _, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_ui_kit::{
    UIApplication, UIPencilInteraction, UIPencilInteractionDelegate, UIPencilInteractionPhase, UIPencilInteractionSqueeze, UIPencilInteractionTap,
    UIPencilPreferredAction, UIView,
};

/// Double taps not yet handed to the UI.
static TAPS: AtomicUsize = AtomicUsize::new(0);
/// Where the last squeeze asked for the menu (points), not yet handed to the UI.
static SQUEEZE: Mutex<Option<egui::Pos2>> = Mutex::new(None);
static CTX: OnceLock<egui::Context> = OnceLock::new();

thread_local! {
    static INSTALLED: RefCell<Option<(Retained<Delegate>, Retained<UIPencilInteraction>)>> = const { RefCell::new(None) };
}

fn wake() {
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

fn tapped(mtm: MainThreadMarker) {
    // The user may have turned the double tap off in Settings.
    if UIPencilInteraction::preferredTapAction(mtm) != UIPencilPreferredAction::Ignore {
        TAPS.fetch_add(1, Ordering::Relaxed);
        wake();
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SolidCraftPencilDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UIPencilInteractionDelegate for Delegate {
        #[unsafe(method(pencilInteraction:didReceiveTap:))]
        fn did_receive_tap(&self, _interaction: &UIPencilInteraction, _tap: &UIPencilInteractionTap) {
            tapped(self.mtm());
        }

        #[unsafe(method(pencilInteraction:didReceiveSqueeze:))]
        fn did_receive_squeeze(&self, _interaction: &UIPencilInteraction, squeeze: &UIPencilInteractionSqueeze) {
            if squeeze.phase() != UIPencilInteractionPhase::Ended {
                return;
            }
            let at = squeeze.hoverPose().map(|pose| pose.location());
            if let (Some(at), Ok(mut slot)) = (at, SQUEEZE.lock()) {
                *slot = Some(egui::pos2(at.x as f32, at.y as f32));
                wake();
            }
        }
    }
);

/// The root view (winit's view), when there is a window.
fn view(mtm: MainThreadMarker) -> Option<Retained<UIView>> {
    #[allow(deprecated)]
    UIApplication::sharedApplication(mtm).keyWindow()?.rootViewController()?.view()
}

/// Listen to the pencil's double tap and squeeze. Safe to call every frame; does the work once.
pub fn install(ctx: &egui::Context) {
    if INSTALLED.with(|i| i.borrow().is_some()) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(view) = view(mtm) else { return };
    let _ = CTX.set(ctx.clone());
    // SAFETY: `new` on an NSObject subclass without ivars returns an owned, initialised object.
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::class(), new] };
    let interaction = UIPencilInteraction::initWithDelegate(mtm.alloc(), ProtocolObject::from_ref(&*delegate));
    view.addInteraction(ProtocolObject::from_ref(&*interaction));
    // The interaction holds its delegate weakly: both are kept for the app's life.
    INSTALLED.with(|i| *i.borrow_mut() = Some((delegate, interaction)));
}

/// Double taps since the last call.
pub fn take_taps() -> usize {
    TAPS.swap(0, Ordering::Relaxed)
}

/// A squeeze's position since the last call.
pub fn take_squeeze() -> Option<egui::Pos2> {
    SQUEEZE.lock().ok().and_then(|mut s| s.take())
}
