//! Windows Shell 缩略图（`IShellItemImageFactory`），可覆盖 HEIC 与常见视频。

use crate::{MediaError, Rgba};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_THUMBNAILONLY,
};

fn err(e: windows::core::Error) -> MediaError {
    MediaError::Decode(e.to_string())
}

/// 取 Shell 缩略图；COM 初始化放在独立线程内，不污染调用线程。
pub fn shell_thumbnail(path: &Path, edge: u32) -> Result<Rgba, MediaError> {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        // SAFETY: 线程内成对调用 CoInitializeEx / CoUninitialize
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let initialized = hr.is_ok();
            let result = shell_thumbnail_inner(&path, edge);
            if initialized {
                CoUninitialize();
            }
            result
        }
    })
    .join()
    .map_err(|_| MediaError::Decode("shell thumbnail thread failed".into()))?
}

unsafe fn shell_thumbnail_inner(path: &Path, edge: u32) -> Result<Rgba, MediaError> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let factory: IShellItemImageFactory =
        SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).map_err(err)?;
    let size = SIZE {
        cx: edge as i32,
        cy: edge as i32,
    };
    // 仅要缩略图：取不到时失败而不是退化成图标，让调用方转用 ffmpeg
    let hbitmap = factory.GetImage(size, SIIGBF_THUMBNAILONLY).map_err(err)?;
    let mut bm = BITMAP::default();
    GetObjectW(
        HGDIOBJ(hbitmap.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut _),
    );
    let (w, h) = (bm.bmWidth, bm.bmHeight);
    let mut out = Err(MediaError::Decode("invalid bitmap dimensions".into()));
    if w > 0 && h > 0 {
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 自上而下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            hbitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        if lines > 0 {
            // BGRA → RGBA；Shell 位图的 alpha 不可靠，统一视为不透明
            for px in pixels.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
                px[3] = 255;
            }
            out = Ok(Rgba {
                width: w as u32,
                height: h as u32,
                pixels,
            });
        }
    }
    let _ = DeleteObject(HGDIOBJ(hbitmap.0));
    out
}
