use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSApplication;
use winit::event_loop::EventLoopProxy;

use crate::event::{Event, EventType};

type Proxy = EventLoopProxy<Event>;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TermustQuitTarget"]
    #[ivars = Proxy]
    pub struct QuitTarget;

    impl QuitTarget {
        #[unsafe(method(requestQuit:))]
        fn request_quit(&self, _sender: Option<&AnyObject>) {
            let _ = self.ivars().send_event(Event::new(EventType::RequestQuit, None));
        }
    }
);

pub fn route_quit(proxy: Proxy) -> Option<Retained<QuitTarget>> {
    let mtm = MainThreadMarker::new()?;
    let app_menu = NSApplication::sharedApplication(mtm).mainMenu()?.itemAtIndex(0)?.submenu()?;
    let item = (0..app_menu.numberOfItems())
        .filter_map(|index| app_menu.itemAtIndex(index))
        .find(|item| item.action() == Some(sel!(terminate:)))?;
    let target: Retained<QuitTarget> = {
        let target = mtm.alloc::<QuitTarget>().set_ivars(proxy);
        unsafe { msg_send![super(target), init] }
    };
    unsafe {
        item.setTarget(Some(&target));
        item.setAction(Some(sel!(requestQuit:)));
    }
    Some(target)
}
