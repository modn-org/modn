#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod backend;
#[cfg(windows)]
mod presentation;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
use backend::Backend;
#[cfg(windows)]
use gpui_kit::assets::Assets;
#[cfg(windows)]
use gpui_kit::{AppContext, WindowBounds, WindowOptions, px, size};

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let backend = Backend::from_args();
    gpui_kit::application().with_assets(Assets).run(move |cx| {
        gpui_kit::init(cx);
        let backend = match backend {
            Ok(backend) => backend,
            Err(error) => {
                let message = format!("Cannot start Modn Desktop: {error:#}");
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| StartupError(message)))
                    .expect("Configuration error window must open");
                cx.activate(true);
                return;
            }
        };
        let options = WindowOptions {
            window_min_size: Some(size(px(980.), px(720.))),
            titlebar: Some(gpui_kit::TitlebarOptions { title: Some(format!("Modn Desktop · PID {}", std::process::id()).into()), ..Default::default() }),
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds::centered(
                None, size(px(1120.), px(800.)), cx,
            ))),
            ..WindowOptions::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| ui::Desktop::new(backend, window, cx));
            let weak = view.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                weak.update(cx, |view, cx| {
                    if view.busy {
                        view.status = "Wait for the current operation before closing. A write cannot be cancelled safely.".into();
                        cx.notify();
                        false
                    } else { true }
                }).unwrap_or(true)
            });
            view
        }).expect("Windows desktop window must open");
        cx.activate(true);
    });
    Ok(())
}

#[cfg(windows)]
struct StartupError(String);

#[cfg(windows)]
impl gpui_kit::Render for StartupError {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement, Styled};
        gpui_kit::div().size_full().p_6().child(gpui_kit::Text::new(
            "startup-error".into(),
            self.0.clone().into(),
        ))
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Modn Desktop requires Windows. Use the modn CLI on this platform.");
    std::process::exit(1);
}
