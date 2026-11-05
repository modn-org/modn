use crate::backend::{self, Backend, Game, Review};
use anyhow::Result;
use gpui_kit::StatefulInteractiveElement;
use gpui_kit::base::Selectable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, PathPromptOptions,
    Render, Role, SharedString, Styled, Subscription, Task, Text, Window, div, px, rgb,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) enum Output {
    Games(Vec<Game>),
    Packages(Value),
    Review(Review),
    Written(Value, Result<Value>),
    Report(Value),
}

#[derive(Clone)]
enum Confirmation {
    Install(Review),
    Uninstall(Game, String),
}

pub(crate) struct Desktop {
    backend: Backend,
    games: Vec<Game>,
    selected: Option<Game>,
    packages: Vec<String>,
    package: Option<String>,
    archive: Entity<InputState>,
    review: Option<Review>,
    confirmation: Option<Confirmation>,
    pub busy: bool,
    pub status: String,
    report: String,
    task: Option<Task<()>>,
    dialog: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl Desktop {
    pub fn new(backend: Backend, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let archive = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Absolute path to a local ZIP or 7z")
        });
        let mut view = Self {
            backend,
            games: vec![],
            selected: None,
            packages: vec![],
            package: None,
            archive: archive.clone(),
            review: None,
            confirmation: None,
            busy: false,
            status: "Loading registered installations...".into(),
            report: "Register installations with the CLI first. Then select one here.".into(),
            task: None,
            dialog: None,
            subscriptions: vec![],
        };
        view.subscriptions
            .push(cx.subscribe(&archive, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.invalidate();
                    this.status = "Archive changed. Inspect and preview again.".into();
                    cx.notify();
                }
            }));
        view.run(cx, |backend| Ok(Output::Games(backend.games()?)));
        view
    }

    fn invalidate(&mut self) {
        self.review = None;
        self.confirmation = None;
    }

    fn run(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(Backend) -> Result<Output> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Working... You can review results when this operation finishes.".into();
        let backend = self.backend.clone();
        let task = cx.background_spawn(async move { work(backend) });
        self.task = Some(cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                this.busy = false;
                this.confirmation = None;
                match result {
                    Ok(Output::Games(games)) => {
                        this.games = games;
                        this.status = "Select a registered installation.".into();
                    }
                    Ok(Output::Packages(packages)) => {
                        this.set_packages(&packages);
                        this.status = "Owned packages refreshed.".into();
                    }
                    Ok(Output::Review(review)) => {
                        this.report = crate::presentation::review_text(&review);
                        this.status = if backend::reviewable(&review.plan) {
                            "Review ready. Check proposed files before installation."
                        } else {
                            "Plan blocked. Resolve blockers before installation."
                        }
                        .into();
                        this.review = Some(review);
                    }
                    Ok(Output::Written(result, packages)) => {
                        this.invalidate();
                        this.report = serde_json::to_string_pretty(&result).unwrap_or_default();
                        match packages {
                            Ok(packages) => {
                                this.set_packages(&packages);
                                this.status = "Change complete. Owned packages refreshed.".into();
                            }
                            Err(error) => {
                                this.packages.clear();
                                this.package = None;
                                this.status = "Write completed. Owned package refresh failed. Run diagnostics and refresh.".into();
                                this.report.push_str(&format!("\n\nInventory refresh failed: {error:#}"));
                            }
                        }
                    }
                    Ok(Output::Report(report)) => {
                        this.report = serde_json::to_string_pretty(&report).unwrap_or_default();
                        this.status = "Diagnostics complete.".into();
                    }
                    Err(error) => {
                        this.invalidate();
                        this.status =
                            "Action refused or failed. Review again before writing.".into();
                        this.report = format!("{error:#}");
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_packages(&mut self, value: &Value) {
        self.packages = value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["id"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        self.package = None;
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.invalidate();
        self.busy = true;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a local ZIP or 7z".into()),
        });
        self.dialog = Some(cx.spawn_in(window, async move |view, cx| {
            let selected = paths.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.busy = false;
                if let Ok(Ok(Some(paths))) = selected {
                    if let Some(path) = paths.first() {
                        this.archive.update(cx, |input, cx| {
                            input.set_value(path.to_string_lossy().to_string(), window, cx)
                        });
                        this.status = "Archive selected. Inspect and preview it.".into();
                    }
                } else {
                    this.status = "File selection cancelled. No changes made.".into();
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some(confirmation) = self.confirmation.take() else {
            return;
        };
        match confirmation {
            Confirmation::Install(review) => {
                if self.selected.as_ref() != Some(&review.game)
                    || Path::new(self.archive.read(cx).value().as_str()) != review.source
                {
                    self.invalidate();
                    self.status = "Selection changed. Review again.".into();
                    return;
                }
                self.run(cx, move |backend| {
                    let written = backend.install(&review)?;
                    Ok(Output::Written(written, backend.read(&review.game, "list")))
                });
            }
            Confirmation::Uninstall(game, id) => {
                if self.selected.as_ref() != Some(&game) || self.package.as_ref() != Some(&id) {
                    self.invalidate();
                    self.status = "Selection changed. Select the owned package again.".into();
                    return;
                }
                self.run(cx, move |backend| {
                    let written = backend.uninstall(&game, &id)?;
                    Ok(Output::Written(written, backend.read(&game, "list")))
                });
            }
        }
    }
}

impl Render for Desktop {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut installations = v_flex().gap_2();
        for (index, game) in self.games.iter().enumerate() {
            let game = game.clone();
            installations = installations.child(
                Button::new(("game", index))
                    .label(format!("{}: {}", game.game_id, game.path.display()))
                    .tooltip(game.path.display().to_string())
                    .selected(self.selected.as_ref() == Some(&game))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.busy {
                            return;
                        }
                        this.invalidate();
                        this.selected = Some(game.clone());
                        this.packages.clear();
                        this.package = None;
                        let game = game.clone();
                        this.run(cx, move |backend| {
                            Ok(Output::Packages(backend.read(&game, "list")?))
                        });
                    })),
            );
        }
        let mut packages = v_flex().gap_2();
        for (index, id) in self.packages.iter().enumerate() {
            let id = id.clone();
            packages = packages.child(
                Button::new(("package", index))
                    .label(id.clone())
                    .selected(self.package.as_ref() == Some(&id))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.busy {
                            return;
                        }
                        this.confirmation = None;
                        this.package = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        let can_install = !self.busy
            && self
                .review
                .as_ref()
                .is_some_and(|review| backend::reviewable(&review.plan));
        let confirmation_label = match &self.confirmation {
            Some(Confirmation::Install(_)) => "Confirm install",
            Some(Confirmation::Uninstall(_, _)) => "Confirm uninstall",
            None => "Confirm change",
        };
        let confirmation_text = match &self.confirmation {
            Some(Confirmation::Install(review)) => format!(
                "Install {} into {}?",
                review.archive.display(),
                review.game.path.display()
            ),
            Some(Confirmation::Uninstall(game, id)) => format!(
                "Strictly uninstall {id} from {}? Modified files cause refusal.",
                game.path.display()
            ),
            None => "Each write requires a reviewed selection and explicit confirmation.".into(),
        };
        v_flex()
            .size_full()
            .font_family("Segoe UI")
            .text_color(rgb(0x182b42))
            .bg(rgb(0xf1f5fa))
            .child(
                v_flex()
                    .px_6()
                    .py_4()
                    .gap_1()
                    .bg(rgb(0xe0eafa))
                    .child(div().text_2xl().child("Modn / Local packages"))
                    .child("Select an installation. Review an archive. Confirm each change."),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        v_flex()
                            .id("sidebar")
                            .overflow_y_scroll()
                            .w(px(330.))
                            .h_full()
                            .p_4()
                            .gap_3()
                            .bg(rgb(0xe8eef5))
                            .child(div().text_lg().child("Registered installations"))
                            .child(installations)
                            .child(
                                Button::new("refresh")
                                    .label("Refresh installations")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.invalidate();
                                        this.selected = None;
                                        this.package = None;
                                        this.packages.clear();
                                        this.run(cx, |backend| Ok(Output::Games(backend.games()?)));
                                    })),
                            )
                            .child(div().mt_4().text_lg().child("Owned packages"))
                            .child(packages)
                            .child(
                                Button::new("list")
                                    .label("Refresh owned packages")
                                    .disabled(self.busy || self.selected.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(game) = this.selected.clone() {
                                            this.confirmation = None;
                                            this.run(cx, move |backend| {
                                                Ok(Output::Packages(backend.read(&game, "list")?))
                                            });
                                        }
                                    })),
                            )
                            .child(
                                Button::new("uninstall")
                                    .label("Uninstall selected package...")
                                    .disabled(self.busy || self.package.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let (Some(game), Some(id)) =
                                            (this.selected.clone(), this.package.clone())
                                        {
                                            this.confirmation =
                                                Some(Confirmation::Uninstall(game, id));
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                Button::new("doctor")
                                    .label("Doctor / diagnostics")
                                    .disabled(self.busy || self.selected.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(game) = this.selected.clone() {
                                            this.run(cx, move |backend| {
                                                Ok(Output::Report(backend.read(&game, "doctor")?))
                                            });
                                        }
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .p_5()
                            .gap_3()
                            .child(div().text_lg().child("Local archive (ZIP or 7z)"))
                            .child(Input::new(&self.archive).disabled(self.busy))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("browse")
                                            .label("Browse...")
                                            .disabled(self.busy)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.browse(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("preview")
                                            .primary()
                                            .label("Inspect and preview")
                                            .disabled(self.busy || self.selected.is_none())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let Some(game) = this.selected.clone() else {
                                                    return;
                                                };
                                                let source = PathBuf::from(
                                                    this.archive.read(cx).value().as_str(),
                                                );
                                                this.invalidate();
                                                this.run(cx, move |backend| {
                                                    Ok(Output::Review(
                                                        backend.review(&game, &source)?,
                                                    ))
                                                });
                                            })),
                                    )
                                    .child(
                                        Button::new("install")
                                            .label("Install reviewed archive...")
                                            .disabled(!can_install)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if let Some(review) = this.review.clone() {
                                                    this.confirmation =
                                                        Some(Confirmation::Install(review));
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            )
                            .child(div().child("Proposed files, blockers and results"))
                            .child(
                                div()
                                    .id("report")
                                    .role(Role::Document)
                                    .aria_label("Proposed files, blockers and results")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .p_3()
                                    .bg(rgb(0xffffff))
                                    .font_family("Consolas")
                                    .text_sm()
                                    .child(Text::new(
                                        "report-text".into(),
                                        SharedString::from(self.report.clone()),
                                    )),
                            )
                            .child(div().child(Text::new(
                                "confirmation-text".into(),
                                confirmation_text.into(),
                            )))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("confirm")
                                            .primary()
                                            .label(confirmation_label)
                                            .disabled(self.busy || self.confirmation.is_none())
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.confirm(cx)),
                                            ),
                                    )
                                    .child(
                                        Button::new("cancel")
                                            .label("Cancel review")
                                            .disabled(self.busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.invalidate();
                                                this.status = "Cancelled. No write started.".into();
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .px_6()
                    .py_3()
                    .bg(rgb(0xd6e3f4))
                    .child(Text::new("status-text".into(), self.status.clone().into())),
            )
    }
}
