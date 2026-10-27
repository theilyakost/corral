use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, RegisterDragDrop, RevokeDragDrop};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Controls::{LVHITTESTINFO, LVM_HITTEST};
use windows::Win32::UI::WindowsAndMessaging::{GetPropW, SendMessageW};
use windows::core::{Interface, Ref, Result, implement, w};

use crate::hook::{ThreadBound, is_hidden};

struct Wrapped {
    hwnd: HWND,
    original: IDropTarget,
}

static WRAPPED: ThreadBound<RefCell<Option<Wrapped>>> = ThreadBound(RefCell::new(None));

#[implement(IDropTarget)]
struct DesktopDrop {
    original: IDropTarget,
    list_view: HWND,
}

impl DesktopDrop {
    fn over_item(&self, pt: &POINTL) -> bool {
        let mut point = POINT { x: pt.x, y: pt.y };
        unsafe {
            _ = ScreenToClient(self.list_view, &mut point);
            let mut info = LVHITTESTINFO { pt: point, ..Default::default() };
            let index = SendMessageW(self.list_view, LVM_HITTEST, None, Some(windows::Win32::Foundation::LPARAM(&mut info as *mut _ as isize)));
            index.0 >= 0 && !is_hidden(index.0 as usize)
        }
    }
}

impl IDropTarget_Impl for DesktopDrop_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { self.original.DragEnter(data.as_ref(), keys, *pt, effect) }
    }

    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { self.original.DragOver(keys, *pt, effect) }
    }

    fn DragLeave(&self) -> Result<()> {
        unsafe { self.original.DragLeave() }
    }

    fn Drop(&self, data: Ref<'_, IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            if self.over_item(pt) {
                return self.original.Drop(data.as_ref(), keys, *pt, effect);
            }
            _ = self.original.DragLeave();
            *effect = DROPEFFECT_NONE;
        }
        Ok(())
    }
}

pub unsafe fn begin(def_view: HWND, list_view: HWND) {
    let Ok(mut wrapped) = WRAPPED.0.try_borrow_mut() else { return };
    if wrapped.is_some() {
        return;
    }
    for hwnd in [list_view, def_view] {
        unsafe {
            let prop = GetPropW(hwnd, w!("OleDropTargetInterface"));
            let raw = prop.0;
            if raw.is_null() {
                continue;
            }
            let Some(original) = IDropTarget::from_raw_borrowed(&raw).cloned() else { continue };
            if RevokeDragDrop(hwnd).is_err() {
                continue;
            }
            let wrapper: IDropTarget = DesktopDrop { original: original.clone(), list_view }.into();
            if RegisterDragDrop(hwnd, &wrapper).is_err() {
                _ = RegisterDragDrop(hwnd, &original);
                return;
            }
            *wrapped = Some(Wrapped { hwnd, original });
            return;
        }
    }
}

pub unsafe fn end() {
    let Some(wrapped) = WRAPPED.0.try_borrow_mut().ok().and_then(|mut w| w.take()) else { return };
    unsafe {
        _ = RevokeDragDrop(wrapped.hwnd);
        _ = RegisterDragDrop(wrapped.hwnd, &wrapped.original);
    }
}
