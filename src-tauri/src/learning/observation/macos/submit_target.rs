use super::*;
use core_foundation_sys::base::CFRetain;
use core_foundation_sys::url::{CFURLGetString, CFURLGetTypeID};

const IDENTIFY_BUDGET: Duration = Duration::from_millis(200);
const MAX_ANCESTORS: usize = 32;
const MAX_COMPOSER_ANCESTORS: usize = 6;
const MAX_COMPOSER_NODES: usize = 40;
const MAX_CHILDREN: isize = 24;
const BUTTON_ATTRIBUTES: &[&str] = &[
    "AXRole",
    "AXEnabled",
    "AXDescription",
    "AXTitle",
    "AXDOMClassList",
    "AXDOMIdentifier",
    "AXIdentifier",
    "AXParent",
];

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementCopyElementAtPosition(
        application: AxRef,
        x: f32,
        y: f32,
        element: *mut AxRef,
    ) -> i32;
}

#[derive(Clone, Copy, PartialEq)]
enum Host {
    Codex,
    ChatGpt,
}

pub(super) struct SubmitTarget {
    host: Host,
    composer: Owned,
    primary_button: Owned,
    web_area: Option<Owned>,
}

impl SubmitTarget {
    /// Call after confirming the injected text. Only metadata in the source
    /// field's bounded ancestor chain and nearest composer is inspected.
    pub(super) fn identify(input: AxRef, bundle_id: &str) -> Option<Self> {
        let host = host_for_bundle(bundle_id)?;
        let deadline = Instant::now() + IDENTIFY_BUDGET;
        let attrs = attributes(
            input,
            &["AXRole", "AXDOMClassList", "AXDOMIdentifier"],
            deadline,
        )?;
        if attribute_string(&attrs, 0).as_deref() != Some("AXTextArea")
            || !(has_class(array_at(&attrs, 1), "ProseMirror")
                || attribute_string(&attrs, 2).as_deref() == Some("prompt-textarea"))
        {
            return None;
        }

        let mut node = attribute(input, "AXParent", deadline)?;
        let mut composer = None;
        for depth in 0..MAX_ANCESTORS {
            let attrs = attributes(node.0, &["AXRole", "AXParent", "AXURL"], deadline)?;
            let role = attribute_string(&attrs, 0)?;
            if role == "AXWebArea" {
                let (composer, primary_button) = composer?;
                return (host == Host::ChatGpt && is_chatgpt_url(array_at(&attrs, 2))).then_some(
                    Self {
                        host,
                        composer,
                        primary_button,
                        web_area: Some(node),
                    },
                );
            }
            if matches!(role.as_str(), "AXWindow" | "AXApplication") {
                return None;
            }
            if composer.is_none() && depth < MAX_COMPOSER_ANCESTORS {
                let mut scan = ComposerScan::default();
                let mut remaining = MAX_COMPOSER_NODES;
                if inspect_composer(node.0, input, host, 0, &mut remaining, deadline, &mut scan)
                    .is_some()
                    && scan.source_found
                    && !scan.other_input
                    && scan.primary_buttons == 1
                {
                    let nearest = retain(node.0)?;
                    let primary_button = scan.primary_button?;
                    if host == Host::Codex {
                        return Some(Self {
                            host,
                            composer: nearest,
                            primary_button,
                            web_area: None,
                        });
                    }
                    composer = Some((nearest, primary_button));
                }
            }
            node = retain(array_at(&attrs, 1)?)?;
        }
        None
    }

    pub(super) fn has_enabled_send(&self, deadline: Instant) -> bool {
        attributes(self.primary_button.0, BUTTON_ATTRIBUTES, deadline)
            .is_some_and(|attrs| enabled_send(self.host, &attrs))
    }

    /// Coordinates are CGEventGetLocation screen points, which share AX hit
    /// testing's top-left origin. Each remote call checks the caller's deadline.
    pub(super) fn send_button_at(
        &self,
        app: AxRef,
        x: f64,
        y: f64,
        deadline: Instant,
    ) -> Option<Owned> {
        if !x.is_finite() || !y.is_finite() || Instant::now() >= deadline {
            return None;
        }
        if let Some(web_area) = &self.web_area {
            let url = attribute(web_area.0, "AXURL", deadline)?;
            if !is_chatgpt_url(Some(url.0)) {
                return None;
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        let mut hit = ptr::null();
        if unsafe { AXUIElementCopyElementAtPosition(app, x as f32, y as f32, &mut hit) } != 0 {
            return None;
        }
        let mut node = Owned::new(hit)?;
        let mut button = None;
        for _ in 0..8 {
            if unsafe { CFEqual(node.0, self.composer.0) } != 0 {
                return button.filter(|_| Instant::now() < deadline);
            }
            let attrs = attributes(node.0, BUTTON_ATTRIBUTES, deadline)?;
            let role = attribute_string(&attrs, 0)?;
            if matches!(role.as_str(), "AXWebArea" | "AXWindow" | "AXApplication") {
                return None;
            }
            if role == "AXButton" {
                if button.is_some() || !enabled_send(self.host, &attrs) {
                    return None;
                }
                button = retain(node.0);
            }
            node = retain(array_at(&attrs, 7)?)?;
        }
        None
    }
}

fn enabled_send(host: Host, attrs: &Owned) -> bool {
    attribute_string(attrs, 0).as_deref() == Some("AXButton")
        && array_at(attrs, 1).is_some_and(boolean_value)
        && primary_control(host, attrs, 4, 5, 6)
        && [2, 3].into_iter().any(|index| {
            attribute_string(attrs, index).is_some_and(|label| send_label(host, &label))
        })
}

fn host_for_bundle(bundle_id: &str) -> Option<Host> {
    match bundle_id {
        "com.openai.codex" => Some(Host::Codex),
        "com.google.Chrome" => Some(Host::ChatGpt),
        id if id.starts_with("com.google.Chrome.app.") => Some(Host::ChatGpt),
        _ => None,
    }
}

fn send_label(host: Host, label: &str) -> bool {
    matches!(
        label,
        "Send"
            | "Send message"
            | "Send prompt"
            | "发送"
            | "发送消息"
            | "发送提示"
            | "傳送"
            | "傳送訊息"
            | "傳送提示"
            | "發送"
    ) || (host == Host::Codex
        && matches!(label, "Queue" | "Steer" | "排队" | "引导" | "排隊" | "引導"))
}

fn primary_control(host: Host, attrs: &Owned, classes: isize, dom_id: isize, id: isize) -> bool {
    // Both the installed Codex and current ChatGPT PWA expose this class.
    // Voice and Stop reuse it, so callers must also require a send label.
    has_class(array_at(attrs, classes), "size-token-button-composer")
        || (host == Host::ChatGpt
            && [dom_id, id].into_iter().any(|index| {
                attribute_string(attrs, index).is_some_and(|id| {
                    matches!(id.as_str(), "composer-submit-button" | "send-button")
                })
            }))
}

#[derive(Default)]
struct ComposerScan {
    source_found: bool,
    other_input: bool,
    primary_buttons: usize,
    primary_button: Option<Owned>,
}

fn inspect_composer(
    element: AxRef,
    source: AxRef,
    host: Host,
    depth: usize,
    remaining: &mut usize,
    deadline: Instant,
    result: &mut ComposerScan,
) -> Option<()> {
    if *remaining == 0 || depth > 5 {
        return None;
    }
    *remaining -= 1;
    if unsafe { CFEqual(element, source) } != 0 {
        result.source_found = true;
        return Some(());
    }
    let attrs = attributes(
        element,
        &[
            "AXRole",
            "AXDOMClassList",
            "AXDOMIdentifier",
            "AXIdentifier",
        ],
        deadline,
    )?;
    match attribute_string(&attrs, 0)?.as_str() {
        "AXTextArea" | "AXTextField" => {
            result.other_input = true;
            return Some(());
        }
        "AXButton" => {
            if primary_control(host, &attrs, 1, 2, 3) {
                result.primary_buttons += 1;
                result.primary_button = retain(element);
            }
            return Some(());
        }
        "AXStaticText" | "AXImage" => return Some(()),
        "AXWebArea" | "AXWindow" | "AXApplication" => return None,
        _ => {}
    }
    if Instant::now() >= deadline {
        return None;
    }
    let name = CFString::new("AXChildren");
    let mut count = 0;
    if unsafe { AXUIElementGetAttributeValueCount(element, name.as_concrete_TypeRef(), &mut count) }
        != 0
    {
        return None;
    }
    if count == 0 {
        return Some(());
    }
    if !(0..=MAX_CHILDREN).contains(&count) || Instant::now() >= deadline {
        return None;
    }
    let mut children = ptr::null();
    if unsafe {
        AXUIElementCopyAttributeValues(element, name.as_concrete_TypeRef(), 0, count, &mut children)
    } != 0
    {
        return None;
    }
    let children = Owned::new(children.cast())?;
    for index in 0..count {
        inspect_composer(
            array_at(&children, index)?,
            source,
            host,
            depth + 1,
            remaining,
            deadline,
            result,
        )?;
    }
    Some(())
}

fn attributes(element: AxRef, names: &[&str], deadline: Instant) -> Option<Owned> {
    if Instant::now() >= deadline {
        return None;
    }
    unsafe { AXUIElementSetMessagingTimeout(element, MESSAGE_TIMEOUT) };
    copy_attributes(element, names)
}

fn attribute(element: AxRef, name: &str, deadline: Instant) -> Option<Owned> {
    if Instant::now() >= deadline {
        return None;
    }
    unsafe { AXUIElementSetMessagingTimeout(element, MESSAGE_TIMEOUT) };
    copy_attribute(element, name)
}

fn attribute_string(attrs: &Owned, index: isize) -> Option<String> {
    array_at(attrs, index).and_then(string_value)
}

fn retain(value: CFTypeRef) -> Option<Owned> {
    if value.is_null() || unsafe { CFGetTypeID(value) != AXUIElementGetTypeID() } {
        return None;
    }
    Owned::new(unsafe { CFRetain(value) })
}

fn is_chatgpt_url(value: Option<CFTypeRef>) -> bool {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return false;
    };
    let value = if unsafe { CFGetTypeID(value) == CFURLGetTypeID() } {
        unsafe { CFURLGetString(value.cast()) }.cast()
    } else {
        value
    };
    string_value(value).is_some_and(|url| chatgpt_origin(&url))
}

fn chatgpt_origin(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("chatgpt.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_chatgpt_https_origin() {
        for url in [
            "https://chatgpt.com/",
            "https://chatgpt.com/c/example",
            "https://chatgpt.com:443/",
        ] {
            assert!(chatgpt_origin(url));
        }
        for url in [
            "http://chatgpt.com/",
            "https://chatgpt.com.evil.test/",
            "https://evil.test/chatgpt.com",
            "https://chatgpt.com:8443/",
            "https://user@chatgpt.com/",
            "file:///chatgpt.com",
        ] {
            assert!(!chatgpt_origin(url));
        }
    }

    #[test]
    fn excludes_other_primary_button_states_and_partial_labels() {
        for host in [Host::Codex, Host::ChatGpt] {
            assert!(send_label(host, "Send"));
            assert!(send_label(host, "发送"));
            for label in [
                "Stop",
                "停止",
                "Resume",
                "继续",
                "开始语音",
                "Attach",
                "发送建议",
                "Send again",
            ] {
                assert!(!send_label(host, label));
            }
        }
        assert!(send_label(Host::Codex, "Queue"));
        assert!(!send_label(Host::ChatGpt, "Queue"));
        assert!(host_for_bundle("com.google.Chrome.app.example").is_some());
        assert!(host_for_bundle("com.google.Chrome.other").is_none());
    }
}
