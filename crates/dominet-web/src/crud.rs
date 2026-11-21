// SPDX-License-Identifier: AGPL-3.0-or-later
//! Composant CRUD generique des pages d'administration : tableau filtrable, formulaire d'ajout / modification,
//! confirmation de suppression, erreurs par champ traduites.
//!
//! Une page decrit sa ressource par des [`FieldSpec`] et fournit deux actions (enregistrer, supprimer) qui appellent
//! ses fonctions serveur ; le composant ne connait ni le type metier ni le reseau. Les libelles sont des *cles* de
//! traduction (`tk!`), resolues a l'affichage pour suivre la langue.

use std::collections::BTreeMap;

use dominet_proto::{Blocked, DeleteOutcome, FieldProblem, ProblemKind, SaveOutcome};
use leptos::prelude::*;
use leptos_fluent::{I18n, move_tr, tr};

use crate::layout::use_hydrated;

/// Valeurs d'un formulaire, par nom de champ. Une case a cocher vaut `"true"` ou `"false"`.
pub type Values = BTreeMap<&'static str, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    TextArea,
    Checkbox,
    /// Nombre entier dans `min..=max`.
    Number {
        min: i64,
        max: i64,
    },
    /// Liste deroulante : (valeur envoyee, cle de traduction du libelle).
    Select(&'static [(&'static str, &'static str)]),
    /// Mot de passe : jamais prerempli, jamais affiche dans le tableau.
    Password,
    /// Colonne du tableau seulement (valeur calculee par le serveur) : absente du formulaire.
    Display,
    /// Liste deroulante dont les choix viennent de la base (groupes, emplacements, appareils) : la page les fournit par le
    /// contexte [`Choices`], sous le nom du champ. Une premiere ligne « aucun » (valeur vide) est toujours proposee.
    Choice,
    /// Liste ordonnee de couples (choix, texte) : par exemple les commandes d'une scene (appareil + commande JSON). Les
    /// choix de la liste deroulante de chaque ligne viennent du contexte [`Choices`] sous le nom du champ. La valeur du
    /// formulaire est la liste en JSON (`[["12","{...}"],...]`).
    Pairs,
    /// Zone de code (script Rhai) en police fixe, avec son panneau d'essai a blanc.
    Code,
}

/// Choix des champs [`FieldKind::Choice`], par nom de champ : (valeur envoyee, texte affiche tel quel, non traduit).
#[derive(Clone, Copy, Debug)]
pub struct Choices(pub Signal<std::collections::BTreeMap<&'static str, Vec<(String, String)>>>);

/// Un champ : nom technique, cle de traduction du libelle, type, valeur initiale d'un nouvel element, obligatoire,
/// longueur maximale, cle de traduction d'un texte d'aide, colonne du tableau.
#[derive(Clone, Debug)]
pub struct FieldSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    pub default: &'static str,
    pub required: bool,
    pub max_len: Option<u32>,
    pub hint: Option<&'static str>,
    /// Champ secret : cle de la valeur ("true"/"false") qui dit s'il est deja enregistre (le secret lui-meme n'est jamais renvoye).
    pub stored_flag: Option<&'static str>,
    /// Modifiable a la creation seulement (affiche en lecture seule ensuite).
    pub create_only: bool,
    pub in_table: bool,
}

impl FieldSpec {
    /// Champ facultatif, sans valeur initiale, hors du tableau ; les methodes ci-dessous precisent le reste.
    #[must_use]
    pub const fn new(name: &'static str, label: &'static str, kind: FieldKind) -> Self {
        Self {
            name,
            label,
            kind,
            default: "",
            required: false,
            max_len: None,
            hint: None,
            stored_flag: None,
            create_only: false,
            in_table: false,
        }
    }

    #[must_use]
    pub const fn default_value(mut self, default: &'static str) -> Self {
        self.default = default;
        self
    }

    #[must_use]
    pub const fn required(mut self) -> Self {
        self.required = true;
        self
    }

    #[must_use]
    pub const fn max(mut self, len: u32) -> Self {
        self.max_len = Some(len);
        self
    }

    #[must_use]
    pub const fn stored_flag(mut self, key: &'static str) -> Self {
        self.stored_flag = Some(key);
        self
    }

    #[must_use]
    pub const fn hint(mut self, key: &'static str) -> Self {
        self.hint = Some(key);
        self
    }

    /// Colonne du tableau.
    #[must_use]
    pub const fn column(mut self) -> Self {
        self.in_table = true;
        self
    }
}

/// Une ligne du tableau : identifiant et valeurs de ses champs.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: i64,
    pub values: Values,
}

/// Issue d'une action d'ecriture, deja reduite a ce que l'interface sait afficher.
#[derive(Clone, Debug, PartialEq)]
pub enum CrudResult {
    Done,
    Problems(Vec<FieldProblem>),
    NotFound,
    InUse,
    /// Refus metier : ni erreur ni champ invalide.
    Blocked(Blocked),
    Failed,
}

impl<T> From<SaveOutcome<T>> for CrudResult {
    fn from(o: SaveOutcome<T>) -> Self {
        match o {
            SaveOutcome::Saved(_) => Self::Done,
            SaveOutcome::Invalid(p) => Self::Problems(p),
            SaveOutcome::NotFound => Self::NotFound,
            SaveOutcome::Blocked(b) => Self::Blocked(b),
        }
    }
}

impl From<DeleteOutcome> for CrudResult {
    fn from(o: DeleteOutcome) -> Self {
        match o {
            DeleteOutcome::Deleted => Self::Done,
            DeleteOutcome::NotFound => Self::NotFound,
            DeleteOutcome::InUse => Self::InUse,
            DeleteOutcome::Blocked(b) => Self::Blocked(b),
        }
    }
}

/// Cles de traduction propres a la ressource.
#[derive(Clone, Copy, Debug)]
pub struct Labels {
    pub title: &'static str,
    pub add: &'static str,
    pub edit: &'static str,
    pub empty: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Banner {
    NotFound,
    InUse,
    Blocked(Blocked),
    Failed,
}

fn defaults(fields: &[FieldSpec]) -> Values {
    fields
        .iter()
        .map(|f| (f.name, f.default.to_string()))
        .collect()
}

fn matches(row: &Row, fields: &[FieldSpec], needle: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    needle.is_empty()
        || fields
            .iter()
            .filter(|f| f.in_table && f.kind != FieldKind::Checkbox)
            .any(|f| {
                row.values
                    .get(f.name)
                    .is_some_and(|v| v.to_lowercase().contains(&needle))
            })
}

/// Etat du formulaire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Editing {
    Closed,
    Creating,
    Updating(i64),
}

/// Etat partage entre le tableau et le formulaire.
#[derive(Clone, Copy)]
struct State {
    editing: RwSignal<Editing>,
    values: RwSignal<Values>,
    problems: RwSignal<Vec<FieldProblem>>,
    banner: RwSignal<Option<Banner>>,
    confirm: RwSignal<Option<i64>>,
}

/// Reagit au resultat des deux actions d'ecriture.
fn wire_results(
    st: State,
    save: Action<(Option<i64>, Values), CrudResult>,
    remove: Action<i64, CrudResult>,
    on_change: Callback<()>,
) {
    Effect::new(move |_| {
        if save.version().get() == 0 {
            return;
        }
        match save.value().get_untracked() {
            Some(CrudResult::Done) => {
                st.editing.set(Editing::Closed);
                on_change.run(());
            }
            Some(CrudResult::Problems(p)) => st.problems.set(p),
            Some(CrudResult::NotFound) => {
                st.editing.set(Editing::Closed);
                st.banner.set(Some(Banner::NotFound));
                on_change.run(());
            }
            Some(CrudResult::Blocked(b)) => st.banner.set(Some(Banner::Blocked(b))),
            Some(CrudResult::Failed | CrudResult::InUse) => st.banner.set(Some(Banner::Failed)),
            None => {}
        }
    });
    Effect::new(move |_| {
        if remove.version().get() == 0 {
            return;
        }
        st.confirm.set(None);
        match remove.value().get_untracked() {
            Some(CrudResult::Done) => on_change.run(()),
            Some(CrudResult::NotFound) => {
                st.banner.set(Some(Banner::NotFound));
                on_change.run(());
            }
            Some(CrudResult::InUse) => st.banner.set(Some(Banner::InUse)),
            Some(CrudResult::Blocked(b)) => st.banner.set(Some(Banner::Blocked(b))),
            _ => st.banner.set(Some(Banner::Failed)),
        }
    });
}

/// Tableau + formulaire. `rows` est la liste courante ; `on_change` est appele apres toute ecriture reussie (ou
/// element disparu) pour la recharger.
#[component]
pub fn CrudTable(
    labels: Labels,
    fields: Vec<FieldSpec>,
    rows: Signal<Vec<Row>>,
    save: Action<(Option<i64>, Values), CrudResult>,
    remove: Action<i64, CrudResult>,
    on_change: Callback<()>,
    /// Affichage conditionnel d'un champ du formulaire : `(nom du champ, valeurs courantes) -> visible`. Par defaut tous
    /// les champs sont visibles ; une page l'utilise pour n'afficher que les champs utiles au type choisi.
    #[prop(optional)]
    field_visible: Option<Callback<(&'static str, Values), bool>>,
) -> impl IntoView {
    let i18n = expect_context::<I18n>();
    let fields = StoredValue::new(fields);
    let st = State {
        editing: RwSignal::new(Editing::Closed),
        values: RwSignal::new(Values::new()),
        problems: RwSignal::new(vec![]),
        banner: RwSignal::new(None),
        confirm: RwSignal::new(None),
    };
    let filter = RwSignal::new(String::new());
    wire_results(st, save, remove, on_change);

    let open_new = move |_| {
        st.values.set(fields.with_value(|f| defaults(f)));
        st.problems.set(vec![]);
        st.banner.set(None);
        st.editing.set(Editing::Creating);
        crate::browser::scroll_to_top();
    };
    let open_edit = Callback::new(move |row: Row| {
        st.values.set(row.values);
        st.problems.set(vec![]);
        st.banner.set(None);
        st.editing.set(Editing::Updating(row.id));
        crate::browser::scroll_to_top();
    });

    view! {
        <div class="vc-page-header">
            <h2>{move || i18n.tr(labels.title)}</h2>
            <div class="vc-header-actions">
                <input
                    type="search"
                    class="vc-search"
                    id="crud-search"
                    placeholder=move || tr!("crud-search")
                    prop:value=move || filter.get()
                    on:input=move |ev| filter.set(event_target_value(&ev))
                />
                <button type="button" id="crud-add" class="vc-btn vc-btn--primary" on:click=open_new>
                    {move || i18n.tr(labels.add)}
                </button>
            </div>
        </div>
        {move || {
            st.banner
                .get()
                .map(|b| {
                    view! {
                        <div class="vc-alert vc-alert--error" role="alert" id="crud-banner">
                            {move || match b {
                                Banner::NotFound => tr!("crud-error-not-found"),
                                Banner::InUse => tr!("crud-error-in-use"),
                                Banner::Blocked(Blocked::LastAdmin) => tr!("crud-error-last-admin"),
                                Banner::Blocked(Blocked::Yourself) => tr!("crud-error-yourself"),
                                Banner::Blocked(Blocked::OwnPassword) => tr!("crud-error-own-password"),
                                Banner::Failed => tr!("crud-error-failed"),
                            }}
                        </div>
                    }
                })
        }}
        <Show when=move || st.editing.get() != Editing::Closed>
            <EditForm labels fields st save field_visible/>
        </Show>
        <DataTable labels fields st rows filter remove open_edit/>
    }
}

/// Formulaire d'ajout ou de modification.
#[component]
fn EditForm(
    labels: Labels,
    fields: StoredValue<Vec<FieldSpec>>,
    st: State,
    save: Action<(Option<i64>, Values), CrudResult>,
    field_visible: Option<Callback<(&'static str, Values), bool>>,
) -> impl IntoView {
    let i18n = expect_context::<I18n>();
    let ready = use_hydrated();
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let id = match st.editing.get_untracked() {
            Editing::Closed => return,
            Editing::Creating => None,
            Editing::Updating(id) => Some(id),
        };
        st.problems.set(vec![]);
        st.banner.set(None);
        save.dispatch((id, st.values.get_untracked()));
    };
    view! {
        <section class="vc-card" id="crud-form-card">
            <h3 class="vc-card-title">
                {move || {
                    if st.editing.get() == Editing::Creating { i18n.tr(labels.add) } else { i18n.tr(labels.edit) }
                }}
            </h3>
            <form method="post" id="crud-form" on:submit=submit>
                {fields
                    .get_value()
                    .into_iter()
                    .map(|f| {
                        // Valeur calculee par le serveur (non modifiable) : montree en lecture seule, seulement si elle existe.
                        if f.kind == FieldKind::Display {
                            let (name, label) = (f.name, f.label);
                            let value = move || st.values.with(|v| v.get(name).cloned().unwrap_or_default());
                            return view! {
                                <Show when=move || !value().is_empty()>
                                    <div class="vc-form-group">
                                        <label>{move || i18n.tr(label)}</label>
                                        <p class="vc-muted" id=format!("crud-display-{name}")>{value}</p>
                                    </div>
                                </Show>
                            }
                            .into_any();
                        }
                        // Memo : le champ n'est recree que si sa visibilite change (pas a chaque frappe ailleurs).
                        let name = f.name;
                        let shown = Memo::new(move |_| {
                            field_visible.is_none_or(|c| c.run((name, st.values.get())))
                        });
                        (move || shown.get().then(|| field_view(f.clone(), st.values, st.problems, st.editing))).into_any()
                    })
                    .collect_view()}
                <div class="vc-form-actions">
                    <button
                        type="submit"
                        id="crud-save"
                        class="vc-btn vc-btn--primary"
                        disabled=move || !ready.get() || save.pending().get()
                    >
                        {move_tr!("crud-save")}
                    </button>
                    <button
                        type="button"
                        id="crud-cancel"
                        class="vc-btn vc-btn--ghost"
                        on:click=move |_| st.editing.set(Editing::Closed)
                    >
                        {move_tr!("crud-cancel")}
                    </button>
                </div>
            </form>
        </section>
    }
}

/// Tableau des lignes, filtre applique.
#[component]
fn DataTable(
    labels: Labels,
    fields: StoredValue<Vec<FieldSpec>>,
    st: State,
    rows: Signal<Vec<Row>>,
    filter: RwSignal<String>,
    remove: Action<i64, CrudResult>,
    open_edit: Callback<Row>,
) -> impl IntoView {
    let i18n = expect_context::<I18n>();
    let columns =
        fields.with_value(|f| f.iter().filter(|c| c.in_table).cloned().collect::<Vec<_>>());
    let head = columns.clone();
    let columns = StoredValue::new(columns);
    let visible = move || {
        let needle = filter.get();
        fields.with_value(|f| {
            rows.get()
                .into_iter()
                .filter(|r| matches(r, f, &needle))
                .collect::<Vec<_>>()
        })
    };
    view! {
        <div class="vc-card">
            <Show
                when=move || !rows.get().is_empty()
                fallback=move || {
                    view! { <p class="vc-muted" id="crud-empty">{move || i18n.tr(labels.empty)}</p> }
                }
            >
                <div class="vc-table-wrap">
                <table class="vc-table" id="crud-table">
                    <thead>
                        <tr>
                            {head
                                .iter()
                                .map(|c| {
                                    let label = c.label;
                                    view! { <th>{move || i18n.tr(label)}</th> }
                                })
                                .collect_view()}
                            <th>{move_tr!("crud-actions")}</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each=visible key=|r| (r.id, r.values.clone()) let:row>
                            {row_view(row, columns.get_value(), st.confirm, remove, open_edit)}
                        </For>
                    </tbody>
                </table>
                </div>
                <Show when=move || visible().is_empty()>
                    <p class="vc-muted" id="crud-no-match">{move_tr!("crud-no-match")}</p>
                </Show>
            </Show>
        </div>
    }
}

/// Message d'erreur affiche sous un champ (rien s'il n'y a pas de probleme pour ce champ).
pub(crate) fn field_error(
    name: &'static str,
    problems: RwSignal<Vec<FieldProblem>>,
) -> impl Fn() -> Option<AnyView> {
    move || {
        problems
            .with(|p| p.iter().find(|x| x.field == name).map(|x| x.kind.clone()))
            .map(|kind| {
                view! {
                    <div class="vc-field-error" id=format!("crud-error-{name}")>
                        {match kind {
                            ProblemKind::Required => tr!("crud-required"),
                            ProblemKind::TooLong { max } => tr!("crud-too-long", {"max" => max}),
                            ProblemKind::Invalid => tr!("crud-invalid"),
                            ProblemKind::Taken => tr!("crud-taken"),
                            ProblemKind::Overlap => tr!("crud-overlap"),
                            ProblemKind::Password(p) => crate::account::problem_text(p),
                            ProblemKind::Syntax(detail) => {
                                tr!("crud-syntax", {"detail" => detail.clone()})
                            }
                            ProblemKind::OutOfRange { min, max } => {
                                tr!("crud-out-of-range", {"min" => min, "max" => max})
                            }
                        }}
                    </div>
                }
                .into_any()
            })
    }
}

/// Un champ de formulaire avec son erreur eventuelle.
#[allow(
    clippy::too_many_lines,
    reason = "a single match on the field types, more readable as one block"
)]
fn field_view(
    f: FieldSpec,
    values: RwSignal<Values>,
    problems: RwSignal<Vec<FieldProblem>>,
    editing: RwSignal<Editing>,
) -> impl IntoView {
    let i18n = expect_context::<I18n>();
    // Non modifiable apres la creation (ex. identifiant d'un compte).
    let locked = f.create_only;
    let readonly = move || locked && editing.get() != Editing::Creating;
    let (name, label, id) = (f.name, f.label, format!("crud-field-{}", f.name));
    let max = f.max_len;
    let required = f.required;
    let stored_flag = f.stored_flag;
    let current = move || values.with(|v| v.get(name).cloned().unwrap_or_default());
    let set = move |value: String| {
        values.update(|v| {
            v.insert(name, value);
        });
    };
    let error = field_error(name, problems);
    let input = match f.kind {
        FieldKind::Text => view! {
            <input
                id=id.clone()
                type="text"
                readonly=readonly
                required=required
                maxlength=max.map(|m| m.to_string())
                prop:value=current
                on:input=move |ev| set(event_target_value(&ev))
            />
        }
        .into_any(),
        FieldKind::TextArea => view! {
            <textarea
                id=id.clone()
                required=required
                maxlength=max.map(|m| m.to_string())
                prop:value=current
                on:input=move |ev| set(event_target_value(&ev))
            ></textarea>
        }
        .into_any(),
        FieldKind::Number { min, max } => view! {
            <input
                id=id.clone()
                type="number"
                step="1"
                min=min.to_string()
                max=max.to_string()
                required=required
                prop:value=current
                on:input=move |ev| set(event_target_value(&ev))
            />
        }
        .into_any(),
        FieldKind::Select(options) => view! {
            <select id=id.clone() on:change=move |ev| set(event_target_value(&ev))>
                {options
                    .iter()
                    .map(|(value, label)| {
                        let value = *value;
                        view! {
                            <option value=value selected=move || current() == value>
                                {move || i18n.tr(label)}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
        }
        .into_any(),
        FieldKind::Choice => choice_select(id.clone(), name, current, set),
        FieldKind::Pairs => pairs_editor(id.clone(), name, current, set),
        FieldKind::Code => code_editor(id.clone(), max, current, set),
        FieldKind::Password => view! {
            <input
                id=id.clone()
                type="password"
                autocomplete="new-password"
                placeholder=move || {
                    stored_flag
                        .is_some_and(|k| values.with(|v| v.get(k).is_some_and(|s| s == "true")))
                        .then_some("\u{25CF}\u{25CF}\u{25CF}\u{25CF}\u{25CF}\u{25CF}\u{25CF}\u{25CF}")
                }
                prop:value=current
                on:input=move |ev| set(event_target_value(&ev))
            />
        }
        .into_any(),
        // Jamais dans un formulaire (voir `EditForm`).
        FieldKind::Display => ().into_any(),
        FieldKind::Checkbox => view! {
            <input
                id=id.clone()
                type="checkbox"
                prop:checked=move || current() == "true"
                on:change=move |ev| set(event_target_checked(&ev).to_string())
            />
        }
        .into_any(),
    };
    view! {
        <div class="vc-form-group" class:vc-form-group--check=f.kind == FieldKind::Checkbox>
            <label for=id>{move || i18n.tr(label)}</label>
            {input}
            {f.hint.map(|h| view! { <div class="vc-hint">{move || i18n.tr(h)}</div> })}
            {error}
        </div>
    }
}

/// Une ligne du tableau, avec ses boutons (la suppression demande confirmation sur place).
fn row_view(
    row: Row,
    columns: Vec<FieldSpec>,
    confirm: RwSignal<Option<i64>>,
    remove: Action<i64, CrudResult>,
    open_edit: Callback<Row>,
) -> impl IntoView {
    let id = row.id;
    let name = columns
        .iter()
        .find(|c| c.kind != FieldKind::Checkbox)
        .and_then(|c| row.values.get(c.name).cloned())
        .unwrap_or_default();
    let cells = columns
        .iter()
        .map(|c| {
            let v = row.values.get(c.name).cloned().unwrap_or_default();
            if c.kind == FieldKind::Checkbox {
                let on = v == "true";
                view! {
                    <td>
                        <span class="vc-badge" class:vc-badge--success=on class:vc-badge--danger=!on>
                            {move || if on { tr!("crud-yes") } else { tr!("crud-no") }}
                        </span>
                    </td>
                }
                .into_any()
            } else if c.kind == FieldKind::Choice {
                let choices = use_context::<Choices>();
                let key = c.name;
                let shown = move || {
                    choices
                        .and_then(|ch| ch.0.with(|m| m.get(key).and_then(|l| l.iter().find(|(value, _)| *value == v).map(|(_, t)| t.clone()))))
                        .unwrap_or_else(|| v.clone())
                };
                let title = shown.clone();
                view! { <td title=title>{shown}</td> }.into_any()
            } else if let FieldKind::Select(options) = c.kind {
                let label = options.iter().find(|(value, _)| *value == v).map(|(_, l)| *l);
                let i18n = expect_context::<I18n>();
                let shown = move || label.map_or_else(|| v.clone(), |l| i18n.tr(l));
                let title = shown.clone();
                view! { <td title=title>{shown}</td> }.into_any()
            } else {
                // `title` : la cellule est tronquee par le style (colonne etroite) ; l'info-bulle garde la valeur complete.
                let title = v.clone();
                view! { <td title=title>{v}</td> }.into_any()
            }
        })
        .collect_view();
    let edit_row = row.clone();
    view! {
        <tr data-id=id>
            {cells}
            <td class="vc-row-actions">
                <Show
                    when=move || confirm.get() == Some(id)
                    fallback=move || {
                        let edit_row = edit_row.clone();
                        view! {
                            <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm crud-edit" on:click=move |_| open_edit.run(edit_row.clone())>
                                {move_tr!("crud-edit")}
                            </button>
                            <button type="button" class="vc-btn vc-btn--danger vc-btn--sm crud-delete" on:click=move |_| confirm.set(Some(id))>
                                {move_tr!("crud-delete")}
                            </button>
                        }
                    }
                >
                    <span class="crud-confirm">{
                        let name = name.clone();
                        move || tr!("crud-confirm-delete", {"name" => name.clone()})
                    }</span>
                    <button type="button" class="vc-btn vc-btn--danger vc-btn--sm crud-confirm-yes" on:click=move |_| { remove.dispatch(id); }>
                        {move_tr!("crud-delete")}
                    </button>
                    <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm crud-confirm-no" on:click=move |_| confirm.set(None)>
                        {move_tr!("crud-cancel")}
                    </button>
                </Show>
            </td>
        </tr>
    }
}

/// Liste deroulante dont les choix viennent du contexte [`Choices`] (premiere ligne : aucun).
fn choice_select(
    id: String,
    name: &'static str,
    current: impl Fn() -> String + Copy + Send + Sync + 'static,
    set: impl Fn(String) + Copy + Send + Sync + 'static,
) -> AnyView {
    let i18n = expect_context::<I18n>();
    let choices = use_context::<Choices>();
    view! {
        <select id=id on:change=move |ev| set(event_target_value(&ev))>
            <option value="" selected=move || current().is_empty()>{move || i18n.tr(tk!("crud-none"))}</option>
            {move || {
                choices
                    .map(|c| c.0.with(|m| m.get(name).cloned().unwrap_or_default()))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(value, text)| {
                        let v = value.clone();
                        view! { <option value=value selected=move || current() == v>{text}</option> }
                    })
                    .collect_view()
            }}
        </select>
    }
    .into_any()
}

/// Une ligne de l'editeur de couples ; `key` est stable pour que la saisie ne perde pas le focus.
#[derive(Clone, Debug, PartialEq)]
struct PairRow {
    key: u32,
    choice: String,
    text: String,
}

/// Valeur du formulaire -> couples (liste vide si le texte n'est pas une liste JSON de couples).
fn parse_pairs(raw: &str) -> Vec<(String, String)> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn encode_pairs(rows: &[PairRow]) -> String {
    let list: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.choice.as_str(), r.text.as_str()))
        .collect();
    serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string())
}

/// Editeur de couples (liste deroulante + texte), avec ajout, suppression et changement d'ordre.
fn pairs_editor(
    id: String,
    name: &'static str,
    current: impl Fn() -> String + Copy + Send + Sync + 'static,
    set: impl Fn(String) + Copy + Send + Sync + 'static,
) -> AnyView {
    let i18n = expect_context::<I18n>();
    let choices = use_context::<Choices>();
    let counter = RwSignal::new(0u32);
    let rows = RwSignal::new(Vec::<PairRow>::new());
    let fresh = move |pairs: Vec<(String, String)>| {
        pairs
            .into_iter()
            .map(|(choice, text)| {
                counter.update(|c| *c += 1);
                PairRow {
                    key: counter.get_untracked(),
                    choice,
                    text,
                }
            })
            .collect::<Vec<_>>()
    };
    // Valeur du formulaire (ouverture d'un autre element) -> lignes ; lignes -> valeur. Chaque sens ne joue que s'il y a
    // une difference, donc pas de boucle.
    Effect::new(move |_| {
        let raw = current();
        let shown = rows.with_untracked(|r| encode_pairs(r));
        if raw != shown {
            rows.set(fresh(parse_pairs(&raw)));
        }
    });
    Effect::new(move |_| {
        let encoded = rows.with(|r| encode_pairs(r));
        if encoded != current() {
            set(encoded);
        }
    });
    let options = move || {
        choices
            .map(|c| c.0.with(|m| m.get(name).cloned().unwrap_or_default()))
            .unwrap_or_default()
    };
    let add = move |_| {
        let row = fresh(vec![(String::new(), String::new())]);
        rows.update(|r| r.extend(row));
    };
    view! {
        <div class="vc-pairs" id=id>
            <For each=move || rows.get() key=|r| r.key let:row>
                <PairsRow row rows options/>
            </For>
            <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm pairs-add" on:click=add>
                {move || i18n.tr(tk!("crud-pairs-add"))}
            </button>
        </div>
    }
    .into_any()
}

/// Une ligne de l'editeur de couples : liste deroulante, texte, monter, descendre, retirer.
#[component]
fn PairsRow(
    row: PairRow,
    rows: RwSignal<Vec<PairRow>>,
    options: impl Fn() -> Vec<(String, String)> + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let i18n = expect_context::<I18n>();
    let key = row.key;
    let (choice, text) = (row.choice, row.text);
    let edit = move |f: Box<dyn FnOnce(&mut PairRow)>| {
        rows.update(|all| {
            if let Some(r) = all.iter_mut().find(|r| r.key == key) {
                f(r);
            }
        });
    };
    let shift = move |delta: isize| {
        rows.update(|all| {
            if let Some(i) = all.iter().position(|r| r.key == key)
                && let Some(j) = i.checked_add_signed(delta)
                && j < all.len()
            {
                all.swap(i, j);
            }
        });
    };
    view! {
        <div class="vc-pairs-row">
            <select
                class="vc-pairs-choice"
                on:change=move |ev| {
                    let v = event_target_value(&ev);
                    edit(Box::new(move |r| r.choice = v));
                }
            >
                <option value="" selected=choice.is_empty()>{move || i18n.tr(tk!("crud-pairs-choose"))}</option>
                {move || {
                    options()
                        .into_iter()
                        .map(|(value, label)| {
                            let selected = value == choice;
                            view! { <option value=value selected=selected>{label}</option> }
                        })
                        .collect_view()
                }}
            </select>
            <input
                type="text"
                class="vc-pairs-text"
                prop:value=text
                on:input=move |ev| {
                    let v = event_target_value(&ev);
                    edit(Box::new(move |r| r.text = v));
                }
            />
            <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm pairs-up" on:click=move |_| shift(-1)>
                "↑"
            </button>
            <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm pairs-down" on:click=move |_| shift(1)>
                "↓"
            </button>
            <button
                type="button"
                class="vc-btn vc-btn--danger vc-btn--sm pairs-remove"
                on:click=move |_| rows.update(|all| all.retain(|r| r.key != key))
            >
                "×"
            </button>
        </div>
    }
}

/// Zone de code et essai a blanc : on donne un topic et un message d'exemple, le serveur dit ce que le script ferait
/// (sans rien publier ni enregistrer).
fn code_editor(
    id: String,
    max: Option<u32>,
    current: impl Fn() -> String + Copy + Send + Sync + 'static,
    set: impl Fn(String) + Copy + Send + Sync + 'static,
) -> AnyView {
    use crate::scripts::dry_run_script;

    let i18n = expect_context::<I18n>();
    let ready = use_hydrated();
    let topic = RwSignal::new(String::new());
    let payload = RwSignal::new("{}".to_string());
    let run = Action::new(|(code, topic, payload): &(String, String, String)| {
        let (code, topic, payload) = (code.clone(), topic.clone(), payload.clone());
        async move { dry_run_script(code, topic, payload).await }
    });
    view! {
        <textarea
            id=id.clone()
            class="vc-code"
            spellcheck="false"
            wrap="off"
            rows="14"
            maxlength=max.map(|m| m.to_string())
            prop:value=current
            on:input=move |ev| set(event_target_value(&ev))
        ></textarea>
        <div class="vc-dryrun" id=format!("{id}-dryrun")>
            <div class="vc-dryrun-form">
                <input
                    type="text"
                    id="dryrun-topic"
                    placeholder=move || i18n.tr(tk!("dryrun-topic"))
                    prop:value=move || topic.get()
                    on:input=move |ev| topic.set(event_target_value(&ev))
                />
                <input
                    type="text"
                    id="dryrun-payload"
                    class="vc-code"
                    placeholder=move || i18n.tr(tk!("dryrun-payload"))
                    prop:value=move || payload.get()
                    on:input=move |ev| payload.set(event_target_value(&ev))
                />
                <button
                    type="button"
                    id="dryrun-run"
                    class="vc-btn vc-btn--ghost vc-btn--sm"
                    disabled=move || !ready.get() || run.pending().get()
                    on:click=move |_| {
                        run.dispatch((current(), topic.get_untracked(), payload.get_untracked()));
                    }
                >
                    {move || i18n.tr(tk!("dryrun-run"))}
                </button>
            </div>
            <p class="vc-hint">{move || i18n.tr(tk!("dryrun-note"))}</p>
            {move || {
                run.value()
                    .get()
                    .map(|r| match r {
                        Err(_) => {
                            view! { <div class="vc-field-error" id="dryrun-error">{move_tr!("error-generic")}</div> }
                                .into_any()
                        }
                        Ok(out) => {
                            let ok = out.error.is_empty();
                            view! {
                                <div id="dryrun-result" class="vc-dryrun-result" class:vc-dryrun-result--error=!ok>
                                    {if ok {
                                        view! { <strong>{move_tr!("dryrun-ok")}</strong> }.into_any()
                                    } else {
                                        view! { <strong id="dryrun-error">{out.error.clone()}</strong> }.into_any()
                                    }}
                                    <ol id="dryrun-effects">
                                        {out.effects.iter().map(|e| view! { <li>{e.clone()}</li> }).collect_view()}
                                    </ol>
                                    {(!out.logs.is_empty())
                                        .then(|| {
                                            view! { <pre id="dryrun-logs">{out.logs.join("\n")}</pre> }
                                        })}
                                    <span class="vc-muted">{format!("{} ms", out.elapsed_ms)}</span>
                                </div>
                            }
                                .into_any()
                        }
                    })
            }}
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_round_trip_through_the_form_value() {
        let rows = vec![
            PairRow {
                key: 1,
                choice: "12".into(),
                text: r#"{"state":"ON"}"#.into(),
            },
            PairRow {
                key: 2,
                choice: String::new(),
                text: "a \"b\" | c".into(),
            },
        ];
        let encoded = encode_pairs(&rows);
        assert_eq!(
            parse_pairs(&encoded),
            [
                ("12".to_string(), r#"{"state":"ON"}"#.to_string()),
                (String::new(), "a \"b\" | c".to_string())
            ]
        );
        assert_eq!(encode_pairs(&[]), "[]");
        assert_eq!(parse_pairs(""), []);
        assert_eq!(parse_pairs("pas du json"), []);
    }
}
