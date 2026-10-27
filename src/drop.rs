use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL,
};
use windows::Win32::Foundation::DRAGDROP_S_DROP;
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget,
    IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{
    BHID_DataObject, CLSID_DragDropHelper, DragQueryFileW, IShellItem, FOLDERID_Desktop, FOLDERID_PublicDesktop, HDROP, IDropTargetHelper,
    KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::{GUID, HRESULT, Interface, Ref, Result, implement};

#[implement(IDropTarget)]
struct GroupDrop {
    hwnd: HWND,
    index: usize,
    header: bool,
    helper: Option<IDropTargetHelper>,
    accepted: std::cell::Cell<bool>,
}

pub fn register(hwnd: HWND, index: usize, header: bool) {
    let helper = unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok() };
    let target: IDropTarget = GroupDrop { hwnd, index, header, helper, accepted: Default::default() }.into();
    unsafe { _ = RegisterDragDrop(hwnd, &target) };
}

#[link(name = "shell32")]
unsafe extern "system" {
    #[link_name = "SHDoDragDrop"]
    fn sh_do_drag_drop(
        hwnd: HWND, data: *mut std::ffi::c_void, source: *mut std::ffi::c_void, allowed: DROPEFFECT,
        effect: *mut DROPEFFECT,
    ) -> HRESULT;
}

pub fn drag_item(hwnd: HWND, item: &IShellItem) -> bool {
    unsafe {
        let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else { return false };
        let allowed = DROPEFFECT(DROPEFFECT_MOVE.0 | DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0);
        let mut effect = DROPEFFECT_NONE;
        sh_do_drag_drop(hwnd, data.as_raw(), std::ptr::null_mut(), allowed, &mut effect) == DRAGDROP_S_DROP
    }
}

pub fn revoke(hwnd: HWND) {
    unsafe { _ = RevokeDragDrop(hwnd) };
}

pub fn desktop_dirs() -> Vec<PathBuf> {
    [FOLDERID_Desktop, FOLDERID_PublicDesktop].iter().filter_map(known_folder).collect()
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let result = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.0 as _));
        result
    }
}

fn dropped_paths(data: &IDataObject) -> Vec<PathBuf> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    unsafe {
        let Ok(mut medium) = data.GetData(&format) else { return Vec::new() };
        let drop = HDROP(medium.u.hGlobal.0);
        let count = DragQueryFileW(drop, u32::MAX, None);
        let mut paths = Vec::new();
        for i in 0..count {
            let mut buf = vec![0u16; DragQueryFileW(drop, i, None) as usize + 1];
            let len = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
            paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        ReleaseStgMedium(&mut medium);
        paths
    }
}

pub fn desktop_names(paths: &[PathBuf], desktops: &[PathBuf]) -> Vec<String> {
    let same = |a: &Path, b: &Path| a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase();
    paths
        .iter()
        .filter(|p| p.parent().is_some_and(|parent| desktops.iter().any(|d| same(parent, d))))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect()
}

fn names(data: Option<&IDataObject>) -> Vec<String> {
    data.map_or_else(Vec::new, |data| desktop_names(&dropped_paths(data), &desktop_dirs()))
}

fn shown_effect(allowed: DROPEFFECT) -> DROPEFFECT {
    [DROPEFFECT_MOVE, DROPEFFECT_LINK, DROPEFFECT_COPY].into_iter().find(|e| allowed.0 & e.0 != 0).unwrap_or(DROPEFFECT_NONE)
}

impl GroupDrop {
    fn effect(&self, effect: *mut DROPEFFECT) -> DROPEFFECT {
        unsafe {
            let shown = if self.accepted.get() { shown_effect(*effect) } else { DROPEFFECT_NONE };
            *effect = shown;
            shown
        }
    }

    fn peek(&self) {
        if self.header {
            crate::hover_header(self.index);
        }
    }
}

fn point(pt: &POINTL) -> POINT {
    POINT { x: pt.x, y: pt.y }
}

impl IDropTarget_Impl for GroupDrop_Impl {
    fn DragEnter(
        &self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT,
    ) -> Result<()> {
        self.accepted.set(!names(data.as_ref()).is_empty());
        let shown = self.effect(effect);
        if let (Some(helper), Some(data)) = (&self.helper, data.as_ref()) {
            unsafe { _ = helper.DragEnter(self.hwnd, data, &point(pt), shown) };
        }
        self.peek();
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let shown = self.effect(effect);
        if let Some(helper) = &self.helper {
            unsafe { _ = helper.DragOver(&point(pt), shown) };
        }
        self.peek();
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        if let Some(helper) = &self.helper {
            unsafe { _ = helper.DragLeave() };
        }
        Ok(())
    }

    fn Drop(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let names = names(data.as_ref());
        if let (Some(helper), Some(data)) = (&self.helper, data.as_ref()) {
            unsafe { _ = helper.Drop(data, &point(pt), shown_effect(*effect)) };
        }
        unsafe { *effect = DROPEFFECT_NONE };
        if !names.is_empty() {
            crate::put_into_group(self.index, names);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_desktop_files_are_taken() {
        let desktops = [PathBuf::from(r"C:\Users\me\Desktop"), PathBuf::from(r"C:\Users\Public\Desktop")];
        let paths = [
            PathBuf::from(r"C:\Users\me\Desktop\a.txt"),
            PathBuf::from(r"c:\users\public\desktop\Tool.lnk"),
            PathBuf::from(r"C:\Users\me\Documents\b.txt"),
            PathBuf::from(r"C:\Users\me\Desktop\sub\c.txt"),
        ];
        assert_eq!(desktop_names(&paths, &desktops), ["a.txt", "Tool.lnk"]);
    }
}
