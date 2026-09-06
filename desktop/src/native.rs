use anyhow::{Context, Result, bail};
use windows::Win32::{
    Foundation::*,
    System::{Com::*, Threading::GetCurrentProcessId},
    UI::{
        Accessibility::*,
        Input::KeyboardAndMouse::*,
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
use windows::core::Interface;

pub struct Automation {
    api: IUIAutomation,
}
pub struct Target {
    window: HWND,
    focus: HWND,
    process: u32,
}

#[derive(Default)]
struct EditorEvidence {
    value_readonly: Option<bool>,
    text_readonly: Option<bool>,
    standard_edit: bool,
    text_edit: bool,
    keyboard_focus: bool,
    focusable: bool,
    active_caret: bool,
    textbox_role: bool,
    terminal: bool,
}
impl EditorEvidence {
    fn editable(&self) -> bool {
        // Explicit read-only flags always win, including on standard Edit controls.
        if self.value_readonly == Some(true) || self.text_readonly == Some(true) {
            return false;
        }
        if self.value_readonly == Some(false) || self.standard_edit || self.text_edit {
            return true;
        }
        self.keyboard_focus
            && self.focusable
            && self.text_readonly == Some(false)
            && (self.active_caret || self.textbox_role || self.terminal)
    }
}

unsafe fn editor_evidence(element: &IUIAutomationElement) -> EditorEvidence {
    use windows::Win32::System::Variant::VT_BOOL;
    unsafe {
        let mut evidence = EditorEvidence {
            value_readonly: element
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|p| p.CurrentIsReadOnly())
                .ok()
                .map(|v| v.as_bool()),
            standard_edit: element.CurrentControlType().ok() == Some(UIA_EditControlTypeId),
            ..Default::default()
        };
        // Avoid probing text providers on the common fast ValuePattern path.
        if evidence.value_readonly.is_some() {
            return evidence;
        }
        evidence.text_edit = element
            .GetCurrentPatternAs::<IUIAutomationTextEditPattern>(UIA_TextEditPatternId)
            .is_ok();
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        {
            if let Ok(value) = pattern
                .DocumentRange()
                .and_then(|r| r.GetAttributeValue(UIA_IsReadOnlyAttributeId))
            {
                // Mixed/unsupported attributes must remain unknown, never convert to false.
                if value.vt() == VT_BOOL {
                    evidence.text_readonly = bool::try_from(&value).ok();
                }
            }
        }
        if evidence.standard_edit || evidence.text_edit || evidence.text_readonly != Some(false) {
            return evidence;
        }
        evidence.keyboard_focus = element.CurrentHasKeyboardFocus().is_ok_and(|v| v.as_bool());
        evidence.focusable = element
            .CurrentIsKeyboardFocusable()
            .is_ok_and(|v| v.as_bool());
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
        {
            let mut active = windows::core::BOOL(0);
            evidence.active_caret = pattern.GetCaretRange(&mut active).is_ok() && active.as_bool();
        }
        let role = element
            .CurrentAriaRole()
            .map(|v| v.to_string())
            .unwrap_or_default();
        evidence.textbox_role = role
            .split_whitespace()
            .any(|r| matches!(r, "textbox" | "searchbox"));
        let class = element
            .CurrentClassName()
            .map(|v| v.to_string())
            .unwrap_or_default();
        let framework = element
            .CurrentFrameworkId()
            .map(|v| v.to_string())
            .unwrap_or_default();
        evidence.terminal = class == "TermControl" && framework == "XAML";
        evidence
    }
}
impl Automation {
    pub fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let api: IUIAutomation2 =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
            api.SetConnectionTimeout(300)?;
            api.SetTransactionTimeout(300)?;
            api.SetAutoSetFocus(false)?;
            Ok(Self { api: api.cast()? })
        }
    }
    pub fn target(&self) -> Result<Target> {
        unsafe {
            let window = GetForegroundWindow();
            if window.is_invalid() {
                bail!("请先选择输入框");
            }
            let mut pid = 0;
            GetWindowThreadProcessId(window, Some(&mut pid));
            if pid == GetCurrentProcessId() {
                bail!("请先把光标放在其他应用的输入框");
            }
            // Capture the stop-time keyboard destination without relying on UIA.
            let focus = native_focus(window);
            if GetForegroundWindow() != window {
                bail!("结束录音时窗口正在切换，请复制结果");
            }
            Ok(Target {
                window,
                focus,
                process: pid,
            })
        }
    }
    pub fn password_focused(&self) -> bool {
        unsafe {
            self.api
                .GetFocusedElement()
                .and_then(|e| e.CurrentIsPassword())
                .map(|v| v.as_bool())
                .unwrap_or(false)
        }
    }
    pub fn matches(&self, target: &Target) -> bool {
        unsafe {
            let mut process = 0;
            GetWindowThreadProcessId(target.window, Some(&mut process));
            process == target.process
                && GetForegroundWindow() == target.window
                && native_focus(target.window) == target.focus
        }
    }
    pub fn inspect_focused_target(&self, expected_pid: u32) -> Result<serde_json::Value> {
        unsafe {
            let window = GetForegroundWindow();
            let mut pid = 0;
            GetWindowThreadProcessId(window, Some(&mut pid));
            if pid != expected_pid {
                bail!("前台进程已变化，未进行诊断");
            }
            let element = self.api.GetFocusedElement()?;
            if element.CurrentIsPassword()?.as_bool() {
                bail!("不诊断密码框");
            }
            let evidence = editor_evidence(&element);
            let old_editable = match evidence.value_readonly {
                Some(value) => !value,
                None => evidence.standard_edit || evidence.text_edit,
            };
            let accepted = self.target().is_ok_and(|target| self.matches(&target));
            Ok(
                serde_json::json!({"foreground_pid":pid,"control_type":element.CurrentControlType()?.0,
                "framework":element.CurrentFrameworkId()?.to_string(),"class":element.CurrentClassName()?.to_string(),
                "value_readonly":evidence.value_readonly,"text_readonly":evidence.text_readonly,
                "active_caret":evidence.active_caret,"textbox_role":evidence.textbox_role,"terminal":evidence.terminal,
                "previously_accepted":old_editable,"previous_guard_editable":evidence.editable(),"accepted":accepted,
                "note":"Read-only metadata check; no text read, input, clipboard changes or submission"}),
            )
        }
    }
    /// Local keyboard destination check; no cross-process accessibility lookup.
    pub fn capture_target_matches(&self, target: &Target) -> bool {
        unsafe {
            GetForegroundWindow() == target.window && native_focus(target.window) == target.focus
        }
    }
    pub fn insert(&self, target: &Target, text: &str) -> Result<()> {
        unsafe {
            if !self.matches(target) {
                bail!("输入目标已变化，结果已保留");
            }
            if modifiers_down() {
                bail!("修饰键仍未松开，结果已保留");
            }
            // Missing UIA support is normal for custom editors. Only a positive
            // password indication blocks delivery; editor identity is not used.
            if self.password_focused() {
                bail!("密码输入框不自动输入，结果已保留");
            }
            let text = text.replace(['\r', '\n'], " ");
            let mut input = Vec::with_capacity(text.len() * 2);
            for code in text.encode_utf16() {
                for flag in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
                    input.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(0),
                                wScan: code,
                                dwFlags: flag,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
            }
            if !self.matches(target) || modifiers_down() {
                bail!("输入目标或按键状态已变化，结果已保留");
            }
            let sent = SendInput(&input, std::mem::size_of::<INPUT>() as i32);
            if sent as usize != input.len() {
                bail!("输入未完整送达，请检查目标应用；管理员窗口可能不支持");
            }
            Ok(())
        }
    }
    pub fn selection(&self) -> Result<String> {
        unsafe {
            let window = GetForegroundWindow();
            let root = self.api.ElementFromHandle(window)?;
            let mut element = self.api.GetFocusedElement()?;
            if element.CurrentProcessId()? != root.CurrentProcessId()? {
                element = root.clone();
            }
            if !element.CurrentIsPassword()?.as_bool()
                && element
                    .CurrentClassName()?
                    .to_string()
                    .eq_ignore_ascii_case("edit")
            {
                if let Ok(text) = classic_edit_selection(element.CurrentNativeWindowHandle()?) {
                    if !text.trim().is_empty() {
                        return Ok(text);
                    }
                }
            }
            let walker = self.api.ControlViewWalker()?;
            let started = std::time::Instant::now();
            for _ in 0..16 {
                if GetForegroundWindow() != window {
                    bail!("前台窗口已变化，请重新选择文字");
                }
                if element
                    .CurrentIsPassword()
                    .map(|v| v.as_bool())
                    .unwrap_or(true)
                {
                    bail!("不读取密码输入框");
                }
                if let Some(text) = selected_text(&element)? {
                    if GetForegroundWindow() != window {
                        bail!("前台窗口已变化，请重新选择文字");
                    }
                    return Ok(text);
                }
                if self
                    .api
                    .CompareElements(&element, &root)
                    .is_ok_and(|v| v.as_bool())
                    || element.CurrentControlType().ok() == Some(UIA_WindowControlTypeId)
                    || started.elapsed() > std::time::Duration::from_millis(1200)
                {
                    break;
                }
                let Ok(parent) = walker.GetParentElement(&element) else {
                    break;
                };
                element = parent;
            }
            // Mouse-selected reading text may be outside the keyboard focus
            // ancestry (e.g. a chat transcript next to its composer). Ask only
            // document providers in this foreground window for THEIR selection.
            if GetForegroundWindow() != window {
                bail!("前台窗口已变化，请重新选择文字");
            }
            let condition = self.api.CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &UIA_DocumentControlTypeId.0.into(),
            )?;
            if let Ok(documents) = root.FindAll(TreeScope_Descendants, &condition) {
                for index in 0..documents.Length()?.min(12) {
                    if GetForegroundWindow() != window
                        || started.elapsed() > std::time::Duration::from_secs(2)
                    {
                        break;
                    }
                    let document = documents.GetElement(index)?;
                    if document.CurrentIsPassword().is_ok_and(|v| !v.as_bool()) {
                        if let Some(text) = selected_text(&document)? {
                            if GetForegroundWindow() != window {
                                bail!("前台窗口已变化，请重新选择文字");
                            }
                            return Ok(text);
                        }
                    }
                }
            }
            bail!("无法读取选区：请先复制，再用托盘中的“朗读剪贴板”")
        }
    }
}

unsafe fn selected_text(element: &IUIAutomationElement) -> Result<Option<String>> {
    unsafe {
        let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        else {
            return Ok(None);
        };
        // Unsupported selection on an intermediate provider must not prevent
        // reaching the document provider that owns the actual selected range.
        let Ok(ranges) = pattern.GetSelection() else {
            return Ok(None);
        };
        if ranges.Length()? > 32 {
            bail!("选区数量过多，请先复制，再使用朗读剪贴板");
        }
        let mut text = String::new();
        for index in 0..ranges.Length()? {
            let part = ranges.GetElement(index)?.GetText(40002)?.to_string();
            if !text.is_empty() && !part.is_empty() {
                text.push('\n');
            }
            text.push_str(&part);
            if text.chars().count() > 20000 {
                bail!("选中文字过长，请选择 20000 字以内");
            }
        }
        Ok((!text.trim().is_empty()).then_some(text))
    }
}

pub fn modifiers_down() -> bool {
    unsafe {
        [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
            .iter()
            .any(|key| GetAsyncKeyState(key.0 as i32) < 0)
    }
}

fn native_focus(window: HWND) -> HWND {
    unsafe {
        let thread_id = GetWindowThreadProcessId(window, None);
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if thread_id != 0 && GetGUIThreadInfo(thread_id, &mut info).is_ok() {
            info.hwndFocus
        } else {
            HWND::default()
        }
    }
}

fn classic_edit_selection(window: HWND) -> Result<String> {
    unsafe {
        // Only system-marshalled messages (< WM_USER), bounded by time and size.
        let mut start = 0u32;
        let mut end = 0u32;
        let mut returned = 0usize;
        if SendMessageTimeoutW(
            window,
            0x00B0,
            WPARAM(&mut start as *mut _ as usize),
            LPARAM(&mut end as *mut _ as isize),
            SMTO_ABORTIFHUNG,
            500,
            Some(&mut returned),
        )
        .0 == 0
        {
            bail!("读取选区超时");
        }
        if end <= start {
            return Ok(String::new());
        }
        if end - start > 20000 || end > 131072 {
            bail!("此选区需要使用复制后朗读");
        }
        let mut buffer = vec![0u16; end as usize + 1];
        if SendMessageTimeoutW(
            window,
            WM_GETTEXT,
            WPARAM(buffer.len()),
            LPARAM(buffer.as_mut_ptr() as isize),
            SMTO_ABORTIFHUNG,
            500,
            Some(&mut returned),
        )
        .0 == 0
        {
            bail!("读取文字超时");
        }
        if returned < end as usize {
            bail!("选区已变化，请重新选择");
        }
        Ok(String::from_utf16_lossy(
            &buffer[start as usize..end as usize],
        ))
    }
}

pub fn hotkey(value: &str) -> Result<(HOT_KEY_MODIFIERS, u32)> {
    let parts: Vec<_> = value.split('+').map(|s| s.trim().to_uppercase()).collect();
    let mut modifiers = MOD_NOREPEAT;
    let mut key = None;
    for part in parts {
        match part.as_str() {
            "CTRL" | "CONTROL" => modifiers |= MOD_CONTROL,
            "ALT" => modifiers |= MOD_ALT,
            "SHIFT" => modifiers |= MOD_SHIFT,
            "SPACE" => {
                if key.replace(VK_SPACE.0 as u32).is_some() {
                    bail!("只能指定一个主键");
                }
            }
            p if p.len() == 1 && p.as_bytes()[0].is_ascii_alphanumeric() => {
                if key.replace(p.as_bytes()[0] as u32).is_some() {
                    bail!("只能指定一个主键");
                }
            }
            _ => bail!("快捷键支持 Ctrl / Alt / Shift 加字母、数字或 Space"),
        }
    }
    if modifiers == MOD_NOREPEAT {
        bail!("快捷键需要 Ctrl、Alt 或 Shift");
    }
    Ok((modifiers, key.context("快捷键缺少主键")?))
}
pub fn register_keys(record: &str, read: &str) -> Result<()> {
    let a = hotkey(record)?;
    let b = hotkey(read)?;
    if a == b {
        bail!("两个快捷键不能相同");
    }
    unsafe {
        RegisterHotKey(None, 1, a.0, a.1).context("听写快捷键已被占用，请在设置中更换")?;
        if let Err(error) = RegisterHotKey(None, 2, b.0, b.1) {
            let _ = UnregisterHotKey(None, 1);
            return Err(error).context("朗读快捷键已被占用，请在设置中更换");
        }
    }
    Ok(())
}
pub fn unregister_keys() {
    unsafe {
        let _ = UnregisterHotKey(None, 1);
        let _ = UnregisterHotKey(None, 2);
    }
}
pub fn poll_keys() -> Vec<i32> {
    unsafe {
        let mut output = Vec::new();
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            if message.message == WM_HOTKEY {
                output.push(message.wParam.0 as i32);
            } else {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        output
    }
}
unsafe extern "system" fn no_activate(
    window: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    unsafe { DefSubclassProc(window, msg, w, l) }
}
pub fn floating(window: HWND) {
    unsafe {
        let old = GetWindowLongPtrW(window, GWL_EXSTYLE);
        SetWindowLongPtrW(
            window,
            GWL_EXSTYLE,
            old | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize,
        );
        let _ = SetWindowSubclass(window, Some(no_activate), 1, 0);
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

pub fn activate_settings(window: HWND) {
    unsafe {
        let old = GetWindowLongPtrW(window, GWL_EXSTYLE);
        SetWindowLongPtrW(
            window,
            GWL_EXSTYLE,
            (old & !(WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize))
                | WS_EX_APPWINDOW.0 as isize,
        );
        let _ = ShowWindow(window, SW_RESTORE);
        let _ = SetWindowPos(
            window,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
        );
        let _ = SetForegroundWindow(window);
    }
}

#[cfg(test)]
mod tests {
    use super::EditorEvidence;
    #[test]
    fn focused_terminal_and_document_editors_are_supported() {
        let mut editor = EditorEvidence {
            text_readonly: Some(false),
            keyboard_focus: true,
            focusable: true,
            terminal: true,
            ..Default::default()
        };
        assert!(editor.editable());
        editor.terminal = false;
        assert!(!editor.editable()); // A writable attribute alone is insufficient.
        editor.textbox_role = true;
        assert!(editor.editable());
        editor.textbox_role = false;
        editor.active_caret = true;
        assert!(editor.editable());
        editor.keyboard_focus = false;
        assert!(!editor.editable());
    }
    #[test]
    fn readonly_and_unknown_text_are_not_editors() {
        for readonly in [Some(true), None] {
            let editor = EditorEvidence {
                text_readonly: readonly,
                keyboard_focus: true,
                focusable: true,
                terminal: true,
                ..Default::default()
            };
            assert!(!editor.editable());
        }
        assert!(
            !EditorEvidence {
                value_readonly: Some(true),
                standard_edit: true,
                ..Default::default()
            }
            .editable()
        );
        assert!(
            !EditorEvidence {
                text_readonly: Some(true),
                standard_edit: true,
                ..Default::default()
            }
            .editable()
        );
    }
    #[test]
    fn hotkeys_need_modifier_and_one_key() {
        assert!(super::hotkey("R").is_err());
        assert!(super::hotkey("Ctrl+R+V").is_err());
        assert!(super::hotkey("Ctrl+Alt+Space").is_ok());
    }
}
