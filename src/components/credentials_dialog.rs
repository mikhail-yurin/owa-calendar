use crate::config::AppConfig;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CredentialsDialogProps {
    pub error_msg: String,
    pub pending_submit: Signal<Option<(String, String)>>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn credentials_dialog(props: CredentialsDialogProps) -> Element {
    let config = AppConfig::load().ok();
    let initial_username = config
        .map(|c| c.calendar.username.clone())
        .unwrap_or_default();

    let mut username = use_signal(|| initial_username);
    let mut password = use_signal(String::new);
    let mut pending_submit = props.pending_submit;
    let on_close = props.on_close;
    let error_msg = props.error_msg.clone();

    let mut do_submit = move || {
        let u = username.read().clone();
        let p = password.read().clone();
        if !u.is_empty() && !p.is_empty() {
            pending_submit.set(Some((u, p)));
        }
    };

    rsx! {
        div {
            style: "position: fixed; inset: 0; background: rgba(0,0,0,0.45); display: flex; align-items: center; justify-content: center; z-index: 1000;",
            onclick: move |_| on_close.call(()),
            div {
                style: "background: #fff; border-radius: 8px; padding: 32px 36px; min-width: 360px; box-shadow: 0 8px 32px rgba(0,0,0,0.18); display: flex; flex-direction: column; gap: 16px;",
                onclick: move |e| e.stop_propagation(),
                div { style: "display: flex; justify-content: space-between; align-items: center;",
                    h2 { style: "margin: 0; font-size: 20px;", "Вход в OWA" }
                    button {
                        style: "background: none; border: none; font-size: 20px; line-height: 1; color: #888; cursor: pointer; padding: 0 4px;",
                        onclick: move |_| on_close.call(()),
                        "✕"
                    }
                }

                if !error_msg.is_empty() {
                    div { style: "background: #fff0f0; border: 1px solid #ffcccc; border-radius: 4px; padding: 8px 12px; color: #cc2200; font-size: 13px;",
                        "{error_msg}"
                    }
                }

                div { style: "display: flex; flex-direction: column; gap: 6px;",
                    label { style: "font-size: 13px; color: #555;", "Логин" }
                    input {
                        r#type: "text",
                        value: "{username}",
                        style: "padding: 8px 10px; border: 1px solid #ccc; border-radius: 4px; font-size: 14px;",
                        oninput: move |e| username.set(e.value()),
                    }
                }

                div { style: "display: flex; flex-direction: column; gap: 6px;",
                    label { style: "font-size: 13px; color: #555;", "Пароль" }
                    input {
                        r#type: "password",
                        value: "{password}",
                        style: "padding: 8px 10px; border: 1px solid #ccc; border-radius: 4px; font-size: 14px;",
                        oninput: move |e| password.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                do_submit();
                            }
                        },
                    }
                }

                button {
                    style: if username.read().is_empty() || password.read().is_empty() { "margin-top: 4px; padding: 10px; background: #a0b8d8; color: #fff; border: none; border-radius: 4px; font-size: 15px; cursor: not-allowed;" } else { "margin-top: 4px; padding: 10px; background: #4a7ebb; color: #fff; border: none; border-radius: 4px; font-size: 15px; cursor: pointer;" },
                    onclick: move |_| do_submit(),
                    "Войти"
                }
            }
        }
    }
}
