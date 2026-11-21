// SPDX-License-Identifier: AGPL-3.0-or-later
//! DTO partages entre le serveur et le front WASM (et contrat Android a terme).
//!
//! Aucune logique ici : des structures `serde` et leurs noms de champs, qui font office de contrat.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleDto {
    Admin,
    User,
}

/// Compte tel que montre a l'interface : jamais de mot de passe ni de condensat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDto {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub mobile: String,
    pub role: RoleDto,
    pub is_active: bool,
    /// RFC 3339 (UTC), absent tant que le compte ne s'est jamais connecte.
    pub last_login_at: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

// `Debug` ne doit jamais montrer le mot de passe.
impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

impl std::fmt::Debug for ChangePasswordRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangePasswordRequest")
            .finish_non_exhaustive()
    }
}

/// Resultat d'une tentative de connexion depuis l'interface (les refus attendus sont des valeurs, pas des erreurs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoginOutcome {
    Success(UserDto),
    /// Nom inconnu, mot de passe faux ou compte desactive : meme reponse pour ne rien reveler.
    InvalidCredentials,
    Locked {
        retry_after_s: u64,
    },
}

/// Motif de refus d'un nouveau mot de passe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PasswordProblem {
    TooShort { min: u32 },
    TooLong { max: u32 },
    AllDigits,
    SameAsUsername,
}

/// Resultat d'un changement de mot de passe depuis l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangePasswordOutcome {
    /// Mot de passe change ; toutes les sessions du compte sont fermees.
    Changed,
    WrongCurrent,
    Rejected(PasswordProblem),
    /// Trop d'essais recents : meme blocage que la connexion.
    Locked {
        retry_after_s: u64,
    },
}

/// Sujet auquel un navigateur peut s'abonner sur `/ws/notifications/` : `notifications`, `system`, `device:<id>`,
/// `camera:<uuid>`. La meme ecriture sert sur le fil, dans le hub du serveur et dans le navigateur.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    /// Messages destines a l'interface (tous les comptes) : toujours abonne a l'ouverture.
    Notifications,
    /// Statistiques de la machine (R2, lot 3).
    System,
    /// Nombre d'alertes en cours (R4, lot 6) : pastille de la barre laterale et page des alertes.
    Alerts,
    /// Etat d'un appareil (identifiant strictement positif).
    Device(i64),
    /// Evenements d'un appareil Android (R5) : presence, SMS, appels, accuses. Reserve a son proprietaire et aux administrateurs.
    Android(i64),
    /// Progression de l'envoi d'un message (R5 lot 5) : reserve a l'expediteur, aux destinataires et aux administrateurs.
    Message(i64),
    /// Resultats de la decouverte ONVIF (R6 lot 3) : reserve aux administrateurs.
    Discovery,
    /// Evenements d'une camera (R6) : refuse tant que la surveillance n'existe pas.
    Camera(uuid::Uuid),
}

impl std::fmt::Display for Topic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Notifications => f.write_str("notifications"),
            Self::System => f.write_str("system"),
            Self::Alerts => f.write_str("alerts"),
            Self::Device(id) => write!(f, "device:{id}"),
            Self::Android(id) => write!(f, "android:{id}"),
            Self::Message(id) => write!(f, "message:{id}"),
            Self::Discovery => f.write_str("discovery"),
            Self::Camera(uuid) => write!(f, "camera:{}", uuid.hyphenated()),
        }
    }
}

/// Texte qui n'est pas un sujet valide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTopic;

impl std::str::FromStr for Topic {
    type Err = InvalidTopic;

    /// Strict : forme canonique uniquement (`device:7`, pas `device:07` ni `Device:7` ni d'espace), pour que deux ecritures
    /// d'un meme sujet ne puissent pas creer deux abonnements.
    fn from_str(s: &str) -> Result<Self, InvalidTopic> {
        let topic = match s {
            "notifications" => Self::Notifications,
            "system" => Self::System,
            "alerts" => Self::Alerts,
            "discovery" => Self::Discovery,
            _ => match s.split_once(':') {
                Some(("device", id)) => Self::Device(id.parse().map_err(|_| InvalidTopic)?),
                Some(("android", id)) => Self::Android(id.parse().map_err(|_| InvalidTopic)?),
                Some(("message", id)) => Self::Message(id.parse().map_err(|_| InvalidTopic)?),
                Some(("camera", uuid)) => {
                    Self::Camera(uuid::Uuid::parse_str(uuid).map_err(|_| InvalidTopic)?)
                }
                _ => return Err(InvalidTopic),
            },
        };
        if matches!(topic, Self::Device(id) | Self::Android(id) | Self::Message(id) if id <= 0)
            || topic.to_string() != s
        {
            return Err(InvalidTopic);
        }
        Ok(topic)
    }
}

/// Etat d'un appareil tel que diffuse a chaque message d'etat (contrat de l'annexe A.1, repris de l'implementation de reference).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceStateDto {
    pub device_id: i64,
    pub friendly_name: String,
    /// Objet JSON des proprietes (`temperature`, `state`, `brightness`...).
    pub state: serde_json::Value,
    pub available: bool,
}

/// Processeur : charge globale et nombre de coeurs logiques.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CpuStats {
    pub percent: f64,
    pub count: u32,
}

/// Memoire vive (les tailles sont en Gio, arrondies a une decimale, comme l'implementation de reference).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MemoryStats {
    pub percent: f64,
    pub used_gb: f64,
    pub total_gb: f64,
    pub available_gb: f64,
}

/// Disque de la racine `/`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DiskStats {
    pub percent: f64,
    pub used_gb: f64,
    pub total_gb: f64,
    pub free_gb: f64,
}

/// Ressources de la machine (sujet `system`, `GET /api/system-stats/`). Format de l'implementation de reference (`cpu`, `memory`,
/// `disk`, `updated_at`) augmente de `temperature` ; `disk` et `temperature` valent `null` quand la machine ne les
/// expose pas (conteneur sans capteur, racine introuvable) : mieux vaut une valeur absente qu'une valeur fausse.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SystemStatsDto {
    pub cpu: CpuStats,
    pub memory: MemoryStats,
    pub disk: Option<DiskStats>,
    /// Degres Celsius du processeur (ou du capteur le plus chaud a defaut).
    pub temperature: Option<f64>,
    /// Secondes Unix (decimales) de la mesure.
    pub updated_at: f64,
}

/// SMS tel que montre a l'interface (champs de `to_dict` de l'implementation de reference).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsDto {
    pub id: i64,
    #[serde(default)]
    pub device_id: i64,
    /// `incoming` ou `outgoing`.
    pub direction: String,
    pub phone_number: String,
    pub contact_name: String,
    pub body: String,
    pub is_read: bool,
    /// RFC 3339 (UTC).
    pub sent_at: String,
    /// Ligne de la file d'envoi d'un SMS sortant (c'est le `message_id` de `sms_sent_ack`).
    #[serde(default)]
    pub outbox_id: Option<i64>,
    /// Etat d'envoi d'un SMS sortant : `pending`, `sent` ou `failed` ; vide pour un SMS recu.
    #[serde(default)]
    pub delivery: String,
    /// Raison de l'echec d'un SMS sortant.
    #[serde(default)]
    pub error: String,
}

/// Entree du journal d'appels (champs de `to_dict` de l'implementation de reference).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallDto {
    pub id: i64,
    #[serde(default)]
    pub device_id: i64,
    /// `incoming`, `outgoing` ou `missed`.
    pub direction: String,
    pub phone_number: String,
    pub contact_name: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Secondes.
    pub duration: i32,
}

/// Ce qui se passe sur un appareil Android (sujet `android:<id>`), au format des messages du navigateur de l'implementation de reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AndroidEvent {
    /// L'appareil se connecte ou se deconnecte ; le dernier etat est conserve par le hub.
    DeviceStatus {
        is_online: bool,
    },
    SmsReceived {
        sms: SmsDto,
    },
    /// Un SMS vient d'etre confie a l'appareil depuis un navigateur (etat `pending` jusqu'a l'accuse).
    SmsQueued {
        sms: SmsDto,
    },
    CallEvent {
        call: CallDto,
        state: String,
    },
    /// Accuse d'envoi d'un SMS : `message_id` est l'identifiant de la file d'envoi (celui de `send_sms`).
    SmsSentAck {
        message_id: i64,
        success: bool,
        error: String,
    },
}

/// Appareil Android tel que montre a l'interface : jamais son jeton.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidDeviceDto {
    pub id: i64,
    pub name: String,
    pub phone_number: String,
    pub owner: String,
    pub is_online: bool,
    /// RFC 3339 (UTC), vide tant que l'appareil ne s'est jamais connecte.
    pub last_seen: String,
}

/// Tableau de bord Android : appareils et activite recente (10 SMS et 10 appels au plus).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidOverviewDto {
    pub devices: Vec<AndroidDeviceDto>,
    pub recent_sms: Vec<SmsDto>,
    pub recent_calls: Vec<CallDto>,
    pub unread: u32,
}

/// Page des SMS d'un appareil (les 100 derniers) ; `device` est absent quand le compte n'a aucun appareil.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidSmsPageDto {
    pub device: Option<AndroidDeviceDto>,
    pub devices: Vec<AndroidDeviceDto>,
    pub sms: Vec<SmsDto>,
}

/// Journal d'appels d'un appareil (les 100 derniers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidCallsPageDto {
    pub device: Option<AndroidDeviceDto>,
    pub devices: Vec<AndroidDeviceDto>,
    pub calls: Vec<CallDto>,
}

/// Enregistrement d'un appareil.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidDeviceInput {
    pub name: String,
    pub phone_number: String,
}

/// Appareil qui vient d'etre cree : son jeton n'est montre **qu'ici**, une seule fois.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAndroidDeviceDto {
    pub device: AndroidDeviceDto,
    pub token: String,
}

// `Debug` ne doit jamais montrer le jeton.
impl std::fmt::Debug for NewAndroidDeviceDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewAndroidDeviceDto")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

/// Resultat de l'envoi d'un SMS depuis le navigateur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendSmsOutcome {
    /// Confie a l'appareil ; l'accuse arrivera en direct.
    Queued(SmsDto),
    Invalid(Vec<FieldProblem>),
    /// L'appareil n'est pas connecte : rien n'a ete enregistre.
    Offline,
    /// Appareil inconnu ou qui n'est pas au compte.
    NotFound,
}

// ------------------------------------------------------------------ messagerie (R5 lot 5)

/// Une personne qu'on peut choisir comme destinataire d'un message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipientChoiceDto {
    pub id: i64,
    pub username: String,
    /// Prenom et nom, ou le nom d'utilisateur.
    pub display_name: String,
    pub has_email: bool,
    pub has_mobile: bool,
}

/// Donnees du formulaire de composition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeFormDto {
    pub users: Vec<RecipientChoiceDto>,
    /// Reglages d'envoi manquants (aucun serveur SMTP, aucun appareil Android en ligne) : simple information avant l'envoi.
    pub email_ready: bool,
    pub android_online: bool,
}

/// Ce que l'administrateur saisit : `channel` vaut `email`, `sms` ou `both` ; `audience` vaut `all`, `staff` ou vide (choix par compte).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeInput {
    pub subject: String,
    pub body: String,
    pub channel: String,
    pub audience: String,
    pub users: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComposeOutcome {
    /// Message enregistre, envoi en cours : `id` est celui de la page de detail.
    Sent {
        id: i64,
        recipients: u32,
    },
    Invalid(Vec<FieldProblem>),
}

/// Message dans une liste (envoyes ou boite de reception).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRowDto {
    pub id: i64,
    pub subject: String,
    pub sender: String,
    pub channel: String,
    pub status: String,
    pub recipients: i32,
    /// RFC 3339 (UTC).
    pub created_at: String,
    /// Statuts du compte connecte pour ce message (boite de reception) ; vides ailleurs.
    pub my_email: String,
    pub my_sms: String,
}

/// Envoi d'un message a un destinataire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryDto {
    pub id: i64,
    pub recipient: String,
    /// `pending`, `sent`, `failed` ou `na`.
    pub email: String,
    pub sms: String,
    pub error: String,
}

/// Page de detail : le tableau complet (`deliveries`) pour l'expediteur et les administrateurs, sinon seulement `mine`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDetailDto {
    pub message: MessageRowDto,
    pub body: String,
    pub full_table: bool,
    pub deliveries: Vec<DeliveryDto>,
    pub mine: Option<DeliveryDto>,
}

/// Pourquoi une demande d'abonnement est refusee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// Sujet mal ecrit.
    Invalid,
    /// Sujet non autorise a ce compte (ou pas encore disponible).
    Forbidden,
    /// Trop d'abonnements sur cette connexion.
    Limit,
}

/// Message du serveur vers le navigateur sur `/ws/notifications/` (une valeur JSON par trame texte).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMsg {
    /// Premiere trame apres l'ouverture : la connexion est authentifiee.
    Hello { username: String },
    /// Battement de coeur : le navigateur doit repondre `pong` avant le suivant, sinon la connexion est fermee.
    Ping,
    /// Message a afficher a l'utilisateur.
    Notification {
        level: NotificationLevel,
        text: String,
    },
    /// Etat d'un appareil (sujet `device:<id>`) ; le dernier etat connu est envoye des l'abonnement.
    DeviceState(DeviceStateDto),
    /// Ressources de la machine (sujet `system`), toutes les quelques secondes.
    SystemStats(SystemStatsDto),
    /// Nombre d'alertes en cours (sujet `alerts`) ; le dernier est conserve.
    AlertCount { firing: u32 },
    /// Evenement d'un appareil Android (sujet `android:<id>`).
    Android { device_id: i64, event: AndroidEvent },
    /// Un envoi de la messagerie a progresse (sujet `message:<id>`) : la page de detail recharge ses statuts. `complete` : plus rien
    /// n'est en attente. Le dernier evenement est conserve par le hub.
    MessageProgress { message_id: i64, complete: bool },
    /// Progression de la decouverte ONVIF (sujet `discovery`, administrateurs).
    Discovery { event: DiscoveryEvent },
    /// Abonnement pris en compte (ou deja actif).
    Subscribed { topic: String },
    /// Abonnement refuse.
    Refused {
        topic: String,
        reason: RefusalReason,
    },
}

/// Un appareil ONVIF qui a repondu a la decouverte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredDto {
    pub ip: String,
    pub port: u16,
    pub name: String,
    pub hardware: String,
    /// Une camera enregistree a deja cette adresse.
    pub known: bool,
}

/// Etape de la decouverte ONVIF.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiscoveryEvent {
    /// Une recherche commence : la liste repart de zero.
    Scanning,
    Found {
        device: DiscoveredDto,
    },
    /// Fin de l'ecoute.
    Done,
    /// La recherche a echoue (message sans secret).
    Failed {
        message: String,
    },
}

/// Etat de la page de decouverte, reconstruit a partir des evenements.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiscoveryView {
    pub scanning: bool,
    pub done: bool,
    pub found: Vec<DiscoveredDto>,
    pub error: Option<String>,
}

impl DiscoveryView {
    pub fn apply(&mut self, event: DiscoveryEvent) {
        match event {
            DiscoveryEvent::Scanning => {
                *self = Self {
                    scanning: true,
                    ..Self::default()
                };
            }
            DiscoveryEvent::Found { device } => {
                if !self.found.iter().any(|d| d.ip == device.ip) {
                    self.found.push(device);
                }
            }
            DiscoveryEvent::Done => {
                self.scanning = false;
                self.done = true;
            }
            DiscoveryEvent::Failed { message } => {
                self.scanning = false;
                self.error = Some(message);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

/// Message du navigateur vers le serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMsg {
    Pong,
    /// S'abonner a un sujet (forme canonique, voir [`Topic`]).
    Subscribe {
        topic: String,
    },
    Unsubscribe {
        topic: String,
    },
}

/// Resultat de la diffusion d'un message a tous les utilisateurs connectes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BroadcastOutcome {
    /// Nombre de connexions ouvertes qui ont recu le message.
    Sent { receivers: u32 },
    /// Texte vide ou trop long.
    Invalid,
}

/// Resultat de l'injection d'un etat d'appareil d'essai.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InjectOutcome {
    /// Nombre d'abonnes qui l'ont recu.
    Sent { receivers: u32 },
    /// Aucun appareil de ce numero.
    InvalidDevice,
    /// Pas un objet JSON, ou trop volumineux (4 Kio au plus).
    InvalidState,
}

/// Un compte tel qu'affiche et edite dans l'administration (jamais de mot de passe ni de condensat).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAdminDto {
    pub id: i64,
    pub username: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub mobile: String,
    pub role: RoleDto,
    pub is_active: bool,
    pub last_login_at: Option<String>,
}

/// Creation (`id` absent) ou modification d'un compte. `username` est ignore en modification (immuable). `password` :
/// obligatoire a la creation, facultatif ensuite (non vide = reinitialisation).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInput {
    pub id: Option<i64>,
    pub username: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub mobile: String,
    pub role: String,
    pub is_active: bool,
    pub password: String,
}

// `Debug` ne doit jamais montrer le mot de passe.
impl std::fmt::Debug for UserInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserInput")
            .field("id", &self.id)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// Un emplacement, tel qu'affiche et edite dans l'administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationDto {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub active: bool,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'un emplacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationInput {
    pub id: Option<i64>,
    pub name: String,
    pub description: String,
    pub active: bool,
}

/// Regle d'alerte Prometheus, telle qu'affichee et editee dans l'administration (R7, ADR 0052).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrometheusRuleDto {
    pub id: i64,
    pub name: String,
    pub group: String,
    pub severity: String,
    pub expr: String,
    pub for_duration: String,
    pub summary: String,
    pub description: String,
    pub eval_interval: String,
    pub active: bool,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'une regle Prometheus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrometheusRuleInput {
    pub id: Option<i64>,
    pub name: String,
    pub group: String,
    pub severity: String,
    pub expr: String,
    pub for_duration: String,
    pub summary: String,
    pub description: String,
    pub eval_interval: String,
    pub active: bool,
}

/// Regle d'alerte Grafana, telle qu'affichee et editee dans l'administration (R7, ADR 0052).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrafanaRuleDto {
    pub id: i64,
    pub uid: String,
    pub title: String,
    pub folder: String,
    pub group: String,
    pub group_interval: String,
    pub datasource_uid: String,
    pub expr: String,
    pub query_window: i32,
    pub for_duration: String,
    pub severity: String,
    pub summary: String,
    pub description: String,
    pub no_data_state: String,
    pub exec_err_state: String,
    pub active: bool,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'une regle Grafana. `uid` vide : tire du titre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrafanaRuleInput {
    pub id: Option<i64>,
    pub uid: String,
    pub title: String,
    pub folder: String,
    pub group: String,
    pub group_interval: String,
    pub datasource_uid: String,
    pub expr: String,
    pub query_window: i32,
    pub for_duration: String,
    pub severity: String,
    pub summary: String,
    pub description: String,
    pub no_data_state: String,
    pub exec_err_state: String,
    pub active: bool,
}

/// Resultat de l'ecriture des fichiers de regles et du rechargement d'un service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSyncDto {
    /// `disabled` (pas de fichier configure), `ok`, `no_url` (fichier ecrit, rechargement a faire a la main),
    /// `reload_failed` (fichier ecrit, service injoignable ou refus) ou `failed` (ecriture impossible).
    pub state: String,
    pub groups: u32,
    pub detail: String,
}

/// Une camera, telle qu'affichee et editee dans l'administration (R6, ADR 0043). Aucun secret : seulement s'il est renseigne.
// Indicateurs independants du formulaire.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraDto {
    pub id: i64,
    /// Identifiant public (32 hexadecimaux) : nom du path MediaMTX.
    pub uuid: String,
    pub name: String,
    pub location_id: Option<i64>,
    pub active: bool,
    pub priority: i32,
    pub notes: String,
    pub proto: String,
    pub ip: String,
    pub port: i32,
    pub path: String,
    pub rtsp_user: String,
    pub has_rtsp_pass: bool,
    pub fps: i32,
    pub rtsp_any_port: bool,
    pub rtsp_transport_tcp: bool,
    pub recording_mode: String,
    pub night_mode_api: String,
    pub web_port: i32,
    pub has_web_api_key: bool,
    pub web_user: String,
    pub has_web_pass: bool,
    pub onvif: bool,
    pub onvif_port: i32,
    pub onvif_user: String,
    pub has_onvif_pass: bool,
    pub ptz: bool,
    pub yolo_enabled: bool,
    /// Classes separees par des virgules ; vide = toutes.
    pub yolo_classes: String,
    /// Confiance minimale en pourcentage (0 a 100).
    pub yolo_min_confidence_pct: i32,
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    /// Derniere connexion (RFC 3339), si connue.
    pub last_seen: Option<String>,
    /// Jeton du profil ONVIF principal (appris par la synchronisation, modifiable).
    #[serde(default)]
    pub main_token: String,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'une camera. Un secret laisse vide reste inchange.
// Indicateurs independants du formulaire.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraInput {
    pub id: Option<i64>,
    pub name: String,
    pub location_id: Option<i64>,
    pub active: bool,
    pub priority: i32,
    pub notes: String,
    pub proto: String,
    pub ip: String,
    pub port: i32,
    pub path: String,
    pub rtsp_user: String,
    pub rtsp_pass: String,
    pub fps: i32,
    pub rtsp_any_port: bool,
    pub rtsp_transport_tcp: bool,
    pub recording_mode: String,
    pub night_mode_api: String,
    pub web_port: i32,
    pub web_api_key: String,
    pub web_user: String,
    pub web_pass: String,
    pub onvif: bool,
    pub onvif_port: i32,
    pub onvif_user: String,
    pub onvif_pass: String,
    /// La camera se pilote en PTZ (coche a la main ; la premiere synchronisation ONVIF la propose).
    pub ptz: bool,
    pub yolo_enabled: bool,
    pub yolo_classes: String,
    pub yolo_min_confidence_pct: i32,
    /// Jeton du profil ONVIF principal ; absent ou vide : la valeur apprise par la synchronisation est gardee.
    #[serde(default)]
    pub main_token: Option<String>,
}

// `Debug` ne doit jamais montrer un mot de passe ni une cle d'API.
impl std::fmt::Debug for CameraInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraInput")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Une camera sur la page de consultation (tout compte connecte) : rien de la configuration du flux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraCardDto {
    pub uuid: String,
    pub name: String,
    pub location: String,
    pub recording_mode: String,
    pub ptz: bool,
    pub last_seen: Option<String>,
}

/// Bilan d'une synchronisation MediaMTX demandee depuis l'administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaMtxSyncDto {
    /// `MEDIAMTX_API_URL` n'est pas renseignee.
    Disabled,
    /// MediaMTX ne repond pas (message deja nettoye de tout secret).
    Unreachable(String),
    Done {
        added: u32,
        replaced: u32,
        removed: u32,
        /// (camera, message) pour ce qui a echoue.
        failed: Vec<(String, String)>,
    },
}

/// Ajout d'une camera par son adresse IP (R6 lot 3). Les identifiants servent au flux et a ONVIF.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddCameraByIp {
    pub ip: String,
    /// Port ONVIF connu ; absent : les ports usuels sont essayes.
    pub onvif_port: Option<u16>,
    pub user: String,
    pub pass: String,
}

// `Debug` ne doit jamais montrer le mot de passe.
impl std::fmt::Debug for AddCameraByIp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddCameraByIp")
            .field("ip", &self.ip)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddByIpOutcome {
    /// `created` : faux si une camera avait deja cette adresse.
    Added {
        id: i64,
        name: String,
        created: bool,
        model: String,
    },
    Invalid(Vec<FieldProblem>),
    /// Aucun service ONVIF sur les ports usuels.
    NoOnvif,
}

/// Resultat d'une synchronisation ONVIF demandee depuis l'administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnvifSyncOutcome {
    Done {
        manufacturer: String,
        model: String,
        firmware: String,
        main_token: String,
        ptz: bool,
        /// URL du flux, mot de passe masque.
        stream_uri: String,
    },
    /// ONVIF n'est pas active sur cette camera.
    Disabled,
    /// Identifiants ONVIF refuses par la camera.
    NotAuthorized,
    /// Camera injoignable ou reponse illisible (message sans secret).
    Failed(String),
    NotFound,
}

/// Un snapshot indexe, tel que la galerie l'affiche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotDto {
    pub id: i64,
    pub camera_uuid: String,
    pub camera_name: String,
    /// RFC 3339.
    pub taken_at: String,
    pub size_bytes: i32,
}

/// Un evenement de camera (mouvement ou detection d'objets), tel que la page des evenements l'affiche.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraEventDto {
    /// `motion` ou `detection`.
    pub kind: String,
    /// RFC 3339, heure du serveur.
    pub at: String,
    pub camera_uuid: String,
    pub camera_name: String,
    /// Part de l'image qui a bouge, en pourcent (mouvement).
    pub motion_pct: Option<f64>,
    /// Classes reconnues, separees par des virgules (detection).
    pub classes: String,
    /// Identifiant de la detection quand une image annotee existe.
    pub frame_id: Option<i64>,
}

/// Reglage global du detecteur d'objets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorDto {
    pub enabled: bool,
    pub save_frames: bool,
}

/// Un segment enregistre, tel que la page des enregistrements l'affiche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingDto {
    pub id: i64,
    /// RFC 3339.
    pub started_at: String,
    pub duration_ms: Option<i64>,
    /// `continuous`, `motion` ou `manual`.
    pub event_type: String,
    /// `pending`, `uploading`, `done` ou `error`.
    pub status: String,
    pub size_bytes: Option<i64>,
    pub error: String,
}

/// Les enregistrements d'une camera pour une journee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingsDayDto {
    pub camera_name: String,
    pub recording_mode: String,
    /// Enregistrement manuel en cours (`None` : camera pas en mode manuel, ou MediaMTX ne le dit pas).
    pub manual_on: Option<bool>,
    pub items: Vec<RecordingDto>,
}

/// Resultat du demarrage / de l'arret de l'enregistrement manuel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManualRecordOutcome {
    Done {
        on: bool,
    },
    NotFound,
    NotManual,
    /// MediaMTX absent ou en panne.
    Unavailable(String),
}

/// Resultat d'une capture demandee depuis la page live (administrateurs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureOutcome {
    Saved(SnapshotDto),
    /// Aucune image recente : le role `video` ne decode pas cette camera (camera eteinte, MediaMTX sans flux...).
    NoImage,
    NotFound,
}

/// Une source MQTT, telle qu'affichee et editee dans l'administration (R3, ADR 0028).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSourceDto {
    pub id: i64,
    pub name: String,
    pub topic_pattern: String,
    pub qos: u8,
    pub active: bool,
    pub log_messages: bool,
    pub auto_device: bool,
    /// Identifiant technique du type d'appareil cree (`sensor`, `switch`...).
    pub auto_device_type: String,
    pub base_topic: String,
    pub handler_code: String,
    pub description: String,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'une source MQTT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSourceInput {
    pub id: Option<i64>,
    pub name: String,
    pub topic_pattern: String,
    pub qos: u8,
    pub active: bool,
    pub log_messages: bool,
    pub auto_device: bool,
    pub auto_device_type: String,
    pub base_topic: String,
    pub handler_code: String,
    pub description: String,
}

/// Ce qui empeche une operation d'administration alors que les donnees sont valides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blocked {
    /// Il doit rester au moins un administrateur actif.
    LastAdmin,
    /// L'administrateur ne peut pas se supprimer, se desactiver ni se retrograder lui-meme.
    Yourself,
    /// Son propre mot de passe se change dans « Mon compte » (la session serait fermee en pleine page).
    OwnPassword,
}

/// Motif de refus d'un champ de formulaire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProblemKind {
    Required,
    TooLong {
        max: u32,
    },
    /// Format ou valeur non admis (le champ affiche une aide sur la forme attendue).
    Invalid,
    OutOfRange {
        min: i64,
        max: i64,
    },
    /// Valeur deja prise (identifiant d'un compte).
    Taken,
    /// L'emplacement d'un widget chevauche un autre widget du tableau.
    Overlap,
    /// Mot de passe refuse par la politique.
    Password(PasswordProblem),
    /// Script qui ne se compile pas : le message de l'analyseur (ligne et colonne comprises).
    Syntax(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldProblem {
    pub field: String,
    pub kind: ProblemKind,
}

/// Une entree de navigation telle qu'editee dans l'administration (l'icone y est un *nom*, pas un SVG).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavbarAdminDto {
    pub id: i64,
    pub icon_name: String,
    pub label: String,
    pub url: String,
    /// `_self` ou `_blank`.
    pub url_target: String,
    pub admin_only: bool,
    /// `top` ou `bottom`.
    pub position: String,
    pub sort_order: i32,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'une entree de navigation. `sort_order` est du texte :
/// l'interface envoie ce que l'utilisateur a saisi et le serveur dit ce qui ne va pas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavbarInput {
    pub id: Option<i64>,
    pub icon_name: String,
    pub label: String,
    pub url: String,
    pub url_target: String,
    pub admin_only: bool,
    pub position: String,
    pub sort_order: String,
}

/// Resultat de l'enregistrement d'un reglage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingOutcome {
    Saved(SettingDto),
    /// Valeur incompatible avec le type du reglage.
    Invalid,
    /// Un secret ne peut pas etre enregistre sans `DOMINET_MASTER_KEY`.
    MasterKeyMissing,
    /// Cle absente du registre.
    UnknownKey,
}

/// Resultat d'un enregistrement depuis l'administration (les refus attendus sont des valeurs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveOutcome<T> {
    Saved(T),
    Invalid(Vec<FieldProblem>),
    /// L'element a ete supprime entre-temps.
    NotFound,
    Blocked(Blocked),
}

impl<T> SaveOutcome<T> {
    /// Converts the saved item (for instance a domain object into its DTO), the refusals unchanged.
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> SaveOutcome<U> {
        match self {
            Self::Saved(item) => SaveOutcome::Saved(f(item)),
            Self::Invalid(list) => SaveOutcome::Invalid(list),
            Self::NotFound => SaveOutcome::NotFound,
            Self::Blocked(b) => SaveOutcome::Blocked(b),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeleteOutcome {
    Deleted,
    NotFound,
    /// Encore reference ailleurs (contrainte de cle etrangere).
    InUse,
    Blocked(Blocked),
}

/// Une entree de la navigation, prete a afficher : l'icone est deja un SVG rendu cote serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavItemDto {
    pub label: String,
    pub url: String,
    pub url_target: String,
    pub icon_svg: String,
}

/// Ce dont la mise en page a besoin : le compte connecte et les entrees de navigation **qu'il a le droit de voir**.
/// Pour un visiteur anonyme, tout est vide (la navigation n'est pas divulguee).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LayoutDto {
    pub user: Option<UserDto>,
    pub nav_top: Vec<NavItemDto>,
    pub nav_bottom: Vec<NavItemDto>,
}

/// Un tableau de bord dans le selecteur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSummaryDto {
    pub id: i64,
    pub name: String,
    pub is_default: bool,
}

/// Un widget prepare pour l'affichage. Les champs `content_html` et `iframe_src` sont calcules par le serveur : le HTML est
/// **deja assaini** et l'adresse d'iframe **deja verifiee** (origine autorisee) ; le navigateur n'a aucune decision de
/// securite a prendre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetDto {
    pub id: i64,
    /// `camera`, `device`, `scene`, `grafana`, `text` ou `grille`.
    pub widget_type: String,
    pub title: String,
    /// Presentation demandee (`""` = automatique), valeurs de l'implementation de reference.
    pub presentation: String,
    pub device_id: Option<i64>,
    pub col: i16,
    pub row: i16,
    pub width: i16,
    pub height: i16,
    /// HTML assaini du widget `text`.
    pub content_html: String,
    /// Adresse a charger dans l'iframe d'un widget Grafana, si elle est autorisee.
    pub iframe_src: Option<String>,
    /// Pourquoi il n'y a pas d'iframe : `not_configured` ou `forbidden` (vide quand `iframe_src` existe).
    pub iframe_problem: String,
    /// Dernier etat connu de l'appareil au moment du rendu (le temps reel prend ensuite le relais).
    pub device: Option<DeviceStateDto>,
    /// Ce que la table des appareils sait de l'appareil (type, libelles, cles masquees, commandes possibles).
    pub device_meta: Option<DeviceMetaDto>,
    /// La scene d'un widget `scene` (absente si elle a ete supprimee ou si ce n'est pas un widget scene).
    pub scene: Option<SceneBriefDto>,
    /// La camera d'un widget `camera` (absente si elle est inconnue ou inactive).
    pub camera: Option<CameraLiveDto>,
}

/// Ce qu'il faut pour afficher le live d'une camera : jamais d'adresse de flux source ni d'identifiants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraLiveDto {
    /// Identifiant public (32 hexadecimaux).
    pub uuid: String,
    pub name: String,
    /// Adresse WHEP de MediaMTX, adaptee a l'hote par lequel le navigateur est arrive.
    pub whep_url: String,
    pub ptz: bool,
    pub recording_mode: String,
    /// Une methode jour / nuit est configuree (les boutons n'ont de sens que dans ce cas ; ils ne sont montres qu'aux administrateurs).
    pub night: bool,
}

/// Resultat d'une bascule jour / nuit / lumiere demandee depuis la page live (administrateurs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NightOutcomeDto {
    /// `details` : ce que la camera a fait, en JSON (la forme de la reference : `mode`, `was`, `value`, `brightness`...).
    Done { action: String, details: String },
    /// Aucune methode jour / nuit n'est configuree pour cette camera.
    NotConfigured,
    /// Camera inconnue ou inactive.
    NotFound,
    /// Action autre que `daynight`, `ircut`, `light`.
    InvalidAction,
    /// La camera n'a pas obei (message sans secret).
    Failed(String),
}

/// Ce qu'un widget scene affiche de sa scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneBriefDto {
    pub id: i64,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub actions: u32,
    pub active: bool,
}

/// Libelle d'une cle d'etat (nom lisible et unite), tel que l'interface l'affiche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDto {
    pub key: String,
    pub name: String,
    pub unit: String,
}

/// Informations d'un appareil utiles a l'affichage de son etat et de ses commandes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeviceMetaDto {
    /// Type stocke (`sensor`, `light`...).
    pub device_type: String,
    pub labels: Vec<LabelDto>,
    /// Cles d'etat a ne pas afficher.
    pub hidden_keys: Vec<String>,
    /// Vrai si le serveur sait publier des commandes (client MQTT demarre).
    pub commandable: bool,
}

/// Un appareil tel que l'administration l'affiche et l'edite (les listes de cles sont separees par des virgules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAdminDto {
    pub id: i64,
    pub friendly_name: String,
    pub description: String,
    pub device_type: String,
    /// Numero du groupe, ou texte vide.
    pub group_id: String,
    /// Numero de l'emplacement, ou texte vide.
    pub location_id: String,
    pub base_topic: String,
    pub active: bool,
    pub icon: String,
    pub image_url: String,
    pub tracked_keys: String,
    pub hidden_keys: String,
    pub grafana_dashboard: bool,
    pub vendor: String,
    pub model_id: String,
    pub ieee: String,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'un appareil : seuls les parametres, pas l'etat ni la decouverte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInput {
    pub id: Option<i64>,
    pub friendly_name: String,
    pub description: String,
    pub device_type: String,
    pub group_id: String,
    pub location_id: String,
    pub base_topic: String,
    pub active: bool,
    pub icon: String,
    pub image_url: String,
    pub tracked_keys: String,
    pub hidden_keys: String,
    pub grafana_dashboard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGroupDto {
    pub id: i64,
    pub name: String,
    pub location_id: String,
    pub description: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGroupInput {
    pub id: Option<i64>,
    pub name: String,
    pub location_id: String,
    pub description: String,
    pub icon: String,
}

/// Libelle d'une cle d'etat tel qu'edite dans l'administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLabelDto {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub icon_name: String,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLabelInput {
    pub id: Option<i64>,
    pub key: String,
    pub name: String,
    pub icon_name: String,
    pub unit: String,
}

/// Choix des listes deroulantes des formulaires (numero, nom).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeviceChoicesDto {
    pub groups: Vec<(i64, String)>,
    pub locations: Vec<(i64, String)>,
    pub devices: Vec<(i64, String)>,
    pub scenes: Vec<(i64, String)>,
}

/// Un appareil sur la page des appareils : identite, dernier etat connu et ce qu'il faut pour l'afficher.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCardDto {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub device_type: String,
    pub group: String,
    pub location: String,
    /// Dernier message recu, en RFC 3339 (texte vide si jamais).
    pub last_seen: String,
    /// Date de l'image televersee (secondes depuis 1970), pour l'adresse et le cache ; `None` sans image.
    pub image_version: Option<i64>,
    pub state: DeviceStateDto,
    pub meta: DeviceMetaDto,
}

/// Page `/automation/` : les appareils actifs filtres, et de quoi remplir les filtres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DevicesPageDto {
    pub devices: Vec<DeviceCardDto>,
    pub groups: Vec<(i64, String)>,
    pub locations: Vec<(i64, String)>,
    /// Types presents (identifiants stockes).
    pub types: Vec<String>,
    pub permit_join: bool,
    pub commandable: bool,
}

/// Historique d'une cle suivie sur les dernieres 24 h.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesDto {
    pub key: String,
    pub name: String,
    pub unit: String,
    /// (instant en secondes depuis 1970, valeur), du plus ancien au plus recent.
    pub points: Vec<(i64, f64)>,
}

/// Page d'un appareil.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceDetailDto {
    pub card: DeviceCardDto,
    pub ieee: String,
    pub vendor: String,
    pub model_id: String,
    pub state_topic: String,
    pub command_topic: String,
    pub tracked_keys: Vec<String>,
    pub active: bool,
    pub series: Vec<SeriesDto>,
    /// Nom du fichier de tableau Grafana deja genere pour cet appareil (admin), s'il y en a un.
    pub grafana_uid: String,
}

/// Une station meteo telle que l'administration l'affiche (latitude et longitude en texte : saisies et affichees telles quelles).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherStationDto {
    pub id: i64,
    pub name: String,
    pub latitude: String,
    pub longitude: String,
    pub altitude: String,
    pub timezone: String,
    pub mqtt_topic_suffix: String,
    pub refresh_minutes: String,
    pub enabled: bool,
    pub description: String,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'une station meteo ; les nombres arrivent en texte et sont lus par le serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherStationInput {
    pub id: Option<i64>,
    pub name: String,
    pub latitude: String,
    pub longitude: String,
    pub altitude: String,
    pub timezone: String,
    pub mqtt_topic_suffix: String,
    pub refresh_minutes: String,
    pub enabled: bool,
    pub description: String,
}

/// Resultat de la generation du tableau Grafana d'un appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrafanaOutcome {
    /// Fichier ecrit (Grafana le recharge en une dizaine de secondes).
    Written,
    /// L'appareil n'a aucune cle suivie : rien a afficher.
    NoTrackedKeys,
    /// `GRAFANA_DASHBOARDS_DIR` n'est pas renseigne, ou le dossier n'est pas inscriptible.
    NotConfigured,
    NotFound,
}

/// Resultat de l'envoi d'une commande a un appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandOutcome {
    /// Publiee sur le broker.
    Sent,
    /// Appareil inconnu ou inactif.
    NotFound,
    /// Commande qui n'est pas un objet JSON raisonnable.
    Invalid,
    /// Pas de lien avec le broker (role `mqtt` absent, broker coupe, file pleine).
    Unavailable,
}

/// Une commande d'une scene, telle qu'affichee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneActionDto {
    pub device_id: i64,
    pub device: String,
    /// Commande JSON, compacte.
    pub payload: String,
}

/// Une scene avec ses commandes dans l'ordre de publication (R4, ADR 0031).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneDto {
    pub id: i64,
    pub name: String,
    pub icon: String,
    pub description: String,
    /// Numero d'emplacement en texte (`""` = aucun), comme les autres formulaires.
    pub location_id: String,
    pub location: String,
    pub active: bool,
    pub actions: Vec<SceneActionDto>,
}

/// Scene a creer ou remplacer, telle que saisie. `actions` : liste de `(numero d'appareil, commande JSON)` en texte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location_id: String,
    pub active: bool,
    #[serde(default)]
    pub actions: Vec<(String, String)>,
}

/// Une commande de scene qui n'est pas partie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneFailureDto {
    pub device: String,
    pub outcome: CommandOutcome,
}

/// Resultat de l'activation d'une scene : les commandes sont publiees dans l'ordre, une en echec n'arrete pas les suivantes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneOutcome {
    Activated {
        sent: u32,
        failed: Vec<SceneFailureDto>,
    },
    /// Scene inconnue ou inactive.
    NotFound,
}

/// Les champs d'une automation tels que le formulaire les saisit (tout en texte : le serveur dit ce qui ne va pas, champ par
/// champ). Ceux qui ne concernent pas le type choisi sont ignores.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutomationFields {
    pub name: String,
    pub enabled: bool,
    pub description: String,
    /// `mqtt_message`, `device_state`, `schedule` ou `solar`.
    pub trigger_type: String,
    pub topic: String,
    /// Objet JSON (facultatif).
    pub payload_filter: String,
    pub friendly_name: String,
    pub key: String,
    pub value: String,
    pub cron: String,
    pub event: String,
    pub latitude: String,
    pub longitude: String,
    pub condition_key: String,
    pub condition_value: String,
    /// `mqtt_publish`, `scene`, `notification` ou `script`.
    pub action_type: String,
    pub action_topic: String,
    /// Objet JSON.
    pub action_payload: String,
    pub message: String,
    pub sms: bool,
    pub scene_id: String,
    pub script: String,
}

/// Creation (`id` absent) ou remplacement (`id` present) d'une automation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationInput {
    pub id: Option<i64>,
    pub fields: AutomationFields,
}

/// Une automation avec ce que l'administration en montre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationDto {
    pub id: i64,
    pub fields: AutomationFields,
    /// Resume lisible du declencheur et de l'action.
    pub trigger_summary: String,
    pub action_summary: String,
    pub trigger_count: i32,
    /// RFC 3339, vide si jamais declenchee.
    pub last_triggered: String,
    /// Prochain passage du planificateur (horaire ou soleil), RFC 3339 ; vide sinon.
    pub next_run: String,
}

/// Une ligne du journal d'execution des automations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRunDto {
    pub automation_id: i64,
    pub automation: String,
    /// RFC 3339.
    pub at: String,
    pub trigger: String,
    /// `ok`, `error`, `skipped` ou `limited`.
    pub status: String,
    pub duration_ms: i32,
    pub message: String,
}

/// Resultat d'un essai a blanc d'un script : rien n'a ete publie, active ni enregistre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunDto {
    /// Erreur de syntaxe ou d'execution ; vide si le script s'est bien termine.
    pub error: String,
    /// Ce que le script aurait fait, dans l'ordre.
    pub effects: Vec<String>,
    /// Lignes de `print` et de `log`.
    pub logs: Vec<String>,
    pub elapsed_ms: u64,
}

/// Une alerte telle que la page `/alerts` l'affiche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertDto {
    pub id: i64,
    /// Vide si la regle a ete supprimee depuis.
    pub rule: String,
    pub device_id: i64,
    pub device: String,
    /// `info`, `warning` ou `critical`.
    pub severity: String,
    /// `firing`, `resolved` ou `acknowledged`.
    pub status: String,
    pub value: String,
    /// RFC 3339.
    pub fired_at: String,
    pub resolved_at: String,
    pub acknowledged_at: String,
    /// Nom d'utilisateur de qui a acquitte.
    pub acknowledged_by: String,
}

/// La page des alertes : le filtre retenu, le nombre d'alertes en cours et les 200 dernieres alertes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AlertsPageDto {
    pub status: String,
    pub firing: u32,
    pub alerts: Vec<AlertDto>,
    /// Les alertes de la plateforme (Prometheus et Grafana), en lecture seule.
    pub platform: PlatformAlertsDto,
}

/// Une alerte en cours chez Prometheus ou Grafana.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformAlertDto {
    /// `prometheus` ou `grafana`.
    pub source: String,
    pub name: String,
    /// `info`, `warning` ou `critical`.
    pub severity: String,
    pub summary: String,
    pub description: String,
    /// RFC 3339, vide si la source ne le donne pas.
    pub since: String,
}

/// Ce que la page sait de la plateforme : l'etat de chaque source (`off` : non surveillee, `down` : injoignable, `ok` : lue) et les
/// alertes en cours, les plus graves d'abord.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlatformAlertsDto {
    pub prometheus: String,
    pub grafana: String,
    pub alerts: Vec<PlatformAlertDto>,
}

/// Une regle d'alerte telle que l'administration l'affiche et l'edite (tout en texte).
// Reflet d'une ligne de formulaire : les canaux et l'activation sont independants.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AlertRuleDto {
    pub id: i64,
    pub name: String,
    pub device_id: String,
    pub device: String,
    pub key: String,
    pub operator: String,
    pub threshold_num: String,
    pub threshold_str: String,
    pub severity: String,
    pub cooldown_minutes: String,
    pub notify_email: bool,
    pub notify_sms: bool,
    pub notify_android: bool,
    pub enabled: bool,
    pub description: String,
    /// Alertes en cours de cette regle.
    pub firing: u32,
}

/// Creation (`id` absent) ou remplacement d'une regle d'alerte.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRuleInput {
    pub id: Option<i64>,
    pub name: String,
    pub device_id: String,
    pub key: String,
    pub operator: String,
    pub threshold_num: String,
    pub threshold_str: String,
    pub severity: String,
    pub cooldown_minutes: String,
    pub notify_email: bool,
    pub notify_sms: bool,
    pub notify_android: bool,
    pub enabled: bool,
    pub description: String,
}

/// Resultat de l'acquittement d'une alerte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AckOutcome {
    Acknowledged,
    /// Alerte inconnue ou deja resolue / acquittee.
    NotFiring,
}

/// Un jour du bilan d'energie tel que la page l'affiche (Wh).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyRowDto {
    pub date: String,
    pub solar_wh: i32,
    pub linky_wh: i32,
    pub pac_wh: i32,
    pub net_wh: i32,
}

/// Un appareil du bilan avec son dernier etat connu (le temps reel prend ensuite le relais par `device:<id>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnergyLiveDto {
    pub id: i64,
    pub name: String,
    pub state: serde_json::Value,
    pub available: bool,
}

/// La page d'energie : jours (du plus ancien au plus recent), graphique SVG deja calcule, et les trois appareils du bilan.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EnergyPageDto {
    pub days: u32,
    pub rows: Vec<EnergyRowDto>,
    /// SVG complet (calcule cote serveur, sans script) ; vide sans donnee.
    pub svg: String,
    pub linky: Option<EnergyLiveDto>,
    pub solar: Option<EnergyLiveDto>,
    pub pac: Option<EnergyLiveDto>,
}

/// Ce que la page d'accueil affiche : les tableaux du compte, celui qui est ouvert et ses widgets.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DashboardPageDto {
    pub dashboards: Vec<DashboardSummaryDto>,
    pub current: Option<DashboardSummaryDto>,
    pub widgets: Vec<WidgetDto>,
}

/// Un tableau tel que l'administration le montre (tous les comptes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardAdminDto {
    pub id: i64,
    pub name: String,
    /// Nom d'utilisateur du proprietaire.
    pub owner: String,
    pub is_default: bool,
}

/// Tableau a creer ou renommer. Le proprietaire se designe par son nom d'utilisateur (il ne change plus apres la creation) ;
/// `is_default` ne peut qu'etre active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardInput {
    pub id: Option<i64>,
    pub name: String,
    pub owner: String,
    pub is_default: bool,
}

/// Un widget tel que l'administration l'edite : les nombres et le JSON voyagent en texte (validation par le serveur).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetAdminDto {
    pub id: i64,
    pub dashboard_id: i64,
    pub widget_type: String,
    pub title: String,
    pub presentation: String,
    pub device_id: String,
    pub scene_id: String,
    pub camera_uuid: String,
    /// Objet JSON, en texte (`{}` si vide).
    pub config: String,
    pub content: String,
    pub col: i16,
    pub row: i16,
    pub width: i16,
    pub height: i16,
}

/// Widget a creer ou a remplacer, tel que saisi (tout en texte : le serveur dit ce qui ne va pas, champ par champ).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetInput {
    pub id: Option<i64>,
    pub dashboard_id: i64,
    pub widget_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub presentation: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub scene_id: String,
    #[serde(default)]
    pub camera_uuid: String,
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub col: String,
    #[serde(default)]
    pub row: String,
    #[serde(default)]
    pub width: String,
    #[serde(default)]
    pub height: String,
}

/// Ce que la page d'administration des widgets affiche : le tableau retenu (celui demande s'il existe, sinon le premier) et ses widgets.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WidgetsPageDto {
    pub current: Option<i64>,
    pub widgets: Vec<WidgetAdminDto>,
}

/// Resultat d'un deplacement de widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveOutcome {
    Moved,
    /// L'emplacement chevauche un autre widget : le widget reste ou il etait.
    Overlap,
    /// Hors de la grille.
    Invalid,
    /// Le widget a ete supprime entre-temps.
    NotFound,
}

/// Un reglage tel que montre a l'interface : la valeur d'un secret n'est **jamais** transmise.
// Quatre indicateurs independants : c'est la forme voulue du contrat JSON.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingDto {
    pub key: String,
    /// `str`, `int`, `float` ou `bool`.
    pub value_type: String,
    pub category: String,
    pub description: String,
    pub is_secret: bool,
    /// Valeur affichable (`********` pour un secret renseigne, vide s'il ne l'est pas).
    pub value: String,
    pub is_set: bool,
    /// Vrai si la valeur par defaut s'applique (rien n'est enregistre en base).
    pub from_default: bool,
    /// Faux si un secret enregistre ne peut pas etre dechiffre (cle maitresse changee).
    pub readable: bool,
    pub updated_at: Option<String>,
}

/// Nouvelle valeur d'un reglage (en clair, y compris pour un secret : ecriture seule).
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct SettingUpdate {
    pub value: String,
}

impl std::fmt::Debug for SettingUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingUpdate").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stats() -> SystemStatsDto {
        SystemStatsDto {
            cpu: CpuStats {
                percent: 12.5,
                count: 4,
            },
            memory: MemoryStats {
                percent: 40.0,
                used_gb: 3.2,
                total_gb: 8.0,
                available_gb: 4.8,
            },
            disk: Some(DiskStats {
                percent: 55.1,
                used_gb: 100.0,
                total_gb: 200.0,
                free_gb: 90.0,
            }),
            temperature: None,
            updated_at: 1_700_000_000.5,
        }
    }

    #[test]
    fn system_stats_keep_the_reference_keys_and_add_temperature() {
        let v = serde_json::to_value(sample_stats()).unwrap();
        let keys = |o: &serde_json::Value| {
            let mut k: Vec<_> = o.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(
            keys(&v),
            ["cpu", "disk", "memory", "temperature", "updated_at"]
        );
        assert_eq!(keys(&v["cpu"]), ["count", "percent"]);
        assert_eq!(
            keys(&v["memory"]),
            ["available_gb", "percent", "total_gb", "used_gb"]
        );
        assert_eq!(
            keys(&v["disk"]),
            ["free_gb", "percent", "total_gb", "used_gb"]
        );
        assert!(v["temperature"].is_null());
    }

    #[test]
    fn system_stats_message_is_flat_and_round_trips() {
        let msg = WsServerMsg::SystemStats(sample_stats());
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "system_stats");
        assert_eq!(json["cpu"]["count"], 4);
        let back: WsServerMsg = serde_json::from_value(json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn role_is_serialized_in_lowercase() {
        assert_eq!(serde_json::to_string(&RoleDto::Admin).unwrap(), "\"admin\"");
        assert_eq!(
            serde_json::from_str::<RoleDto>("\"user\"").unwrap(),
            RoleDto::User
        );
    }

    #[test]
    fn user_dto_field_names_are_the_contract() {
        let u = UserDto {
            id: 1,
            username: "bob".into(),
            display_name: "Bob".into(),
            email: String::new(),
            mobile: String::new(),
            role: RoleDto::User,
            is_active: true,
            last_login_at: None,
        };
        let v = serde_json::to_value(&u).unwrap();
        for key in [
            "id",
            "username",
            "display_name",
            "email",
            "mobile",
            "role",
            "is_active",
            "last_login_at",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert!(v.get("password").is_none() && v.get("password_hash").is_none());
    }

    #[test]
    fn setting_update_never_appears_in_debug_and_setting_dto_has_no_raw_value_field() {
        let u = SettingUpdate {
            value: "cle-secrete".into(),
        };
        assert!(!format!("{u:?}").contains("cle-secrete"));
        let d = SettingDto {
            key: "SMS_FREE_KEY".into(),
            value_type: "str".into(),
            category: "Notifications".into(),
            description: String::new(),
            is_secret: true,
            value: "********".into(),
            is_set: true,
            from_default: false,
            readable: true,
            updated_at: None,
        };
        let v = serde_json::to_value(&d).unwrap();
        for key in [
            "key",
            "value_type",
            "category",
            "description",
            "is_secret",
            "value",
            "is_set",
            "from_default",
            "readable",
            "updated_at",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn passwords_never_appear_in_debug() {
        let l = LoginRequest {
            username: "bob".into(),
            password: "secret-pw".into(),
        };
        let c = ChangePasswordRequest {
            current_password: "old-pw".into(),
            new_password: "new-pw".into(),
        };
        let dump = format!("{l:?} {c:?}");
        for s in ["secret-pw", "old-pw", "new-pw"] {
            assert!(!dump.contains(s), "{s}");
        }
        assert!(dump.contains("bob"));
    }

    #[test]
    fn websocket_messages_have_a_stable_wire_format() {
        let hello = serde_json::to_string(&WsServerMsg::Hello {
            username: "bob".into(),
        })
        .unwrap();
        assert_eq!(hello, r#"{"type":"hello","username":"bob"}"#);
        assert_eq!(
            serde_json::to_string(&WsServerMsg::Ping).unwrap(),
            r#"{"type":"ping"}"#
        );
        let n = WsServerMsg::Notification {
            level: NotificationLevel::Warning,
            text: "x".into(),
        };
        assert_eq!(
            serde_json::to_string(&n).unwrap(),
            r#"{"type":"notification","level":"warning","text":"x"}"#
        );
        assert_eq!(
            serde_json::from_str::<WsClientMsg>(r#"{"type":"pong"}"#).unwrap(),
            WsClientMsg::Pong
        );
        assert!(serde_json::from_str::<WsClientMsg>(r#"{"type":"nope"}"#).is_err());
    }

    #[test]
    fn android_events_have_a_stable_json_shape() {
        let msg = WsServerMsg::Android {
            device_id: 3,
            event: AndroidEvent::SmsSentAck {
                message_id: 9,
                success: false,
                error: "no signal".into(),
            },
        };
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "android", "device_id": 3, "event": {"kind": "sms_sent_ack", "message_id": 9, "success": false, "error": "no signal"}})
        );
        assert_eq!(serde_json::from_value::<WsServerMsg>(json).unwrap(), msg);
        let status =
            serde_json::to_string(&AndroidEvent::DeviceStatus { is_online: true }).unwrap();
        assert_eq!(status, r#"{"kind":"device_status","is_online":true}"#);
    }

    #[test]
    fn topics_round_trip_and_only_the_canonical_form_is_accepted() {
        let uuid = uuid::Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap();
        for (text, topic) in [
            ("notifications", Topic::Notifications),
            ("system", Topic::System),
            ("alerts", Topic::Alerts),
            ("device:7", Topic::Device(7)),
            ("device:9223372036854775807", Topic::Device(i64::MAX)),
            ("android:3", Topic::Android(3)),
            ("message:12", Topic::Message(12)),
            ("discovery", Topic::Discovery),
            (
                "camera:123e4567-e89b-12d3-a456-426614174000",
                Topic::Camera(uuid),
            ),
        ] {
            assert_eq!(text.parse::<Topic>(), Ok(topic.clone()), "{text}");
            assert_eq!(topic.to_string(), text);
        }
        for bad in [
            "",
            " system",
            "system ",
            "System",
            "device",
            "device:",
            "device:0",
            "device:-3",
            "device:07",
            "device:+7",
            "device:7:8",
            "device:abc",
            "android:0",
            "message:0",
            "message:012",
            "Message:1",
            "android:03",
            "android:-1",
            "android:",
            "Android:3",
            "device: 7",
            "camera:xyz",
            "camera:123E4567-E89B-12D3-A456-426614174000",
            "camera:123e4567e89b12d3a456426614174000",
            "devices:7",
            "notifications:1",
            "all",
            "*",
            "#",
        ] {
            assert_eq!(bad.parse::<Topic>(), Err(InvalidTopic), "{bad:?}");
        }
    }

    #[test]
    fn the_discovery_view_follows_the_events() {
        let device = |ip: &str| DiscoveredDto {
            ip: ip.into(),
            port: 80,
            name: String::new(),
            hardware: String::new(),
            known: false,
        };
        let mut v = DiscoveryView::default();
        v.apply(DiscoveryEvent::Scanning);
        v.apply(DiscoveryEvent::Found {
            device: device("10.0.0.2"),
        });
        v.apply(DiscoveryEvent::Found {
            device: device("10.0.0.2"),
        });
        v.apply(DiscoveryEvent::Found {
            device: device("10.0.0.3"),
        });
        assert!(v.scanning && !v.done);
        assert_eq!(v.found.len(), 2, "une adresse n'est listee qu'une fois");
        v.apply(DiscoveryEvent::Done);
        assert!(!v.scanning && v.done);
        // Une nouvelle recherche repart de zero.
        v.apply(DiscoveryEvent::Scanning);
        assert!(v.found.is_empty() && !v.done && v.scanning);
        v.apply(DiscoveryEvent::Failed {
            message: "x".into(),
        });
        assert_eq!((v.scanning, v.error.as_deref()), (false, Some("x")));
    }

    #[test]
    fn the_new_messages_have_the_wire_format_of_the_reference_version() {
        let state = WsServerMsg::DeviceState(DeviceStateDto {
            device_id: 42,
            friendly_name: "Bureau".into(),
            state: serde_json::json!({"temperature": 22.5, "humidity": 48}),
            available: true,
        });
        // Contrat de l'annexe A.1 : champs a plat a cote de `type`.
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::json!({"type": "device_state", "device_id": 42, "friendly_name": "Bureau",
                               "state": {"temperature": 22.5, "humidity": 48}, "available": true})
        );
        assert_eq!(
            serde_json::from_str::<WsServerMsg>(&serde_json::to_string(&state).unwrap()).unwrap(),
            state
        );
        assert_eq!(
            serde_json::to_string(&WsServerMsg::Subscribed {
                topic: "device:7".into()
            })
            .unwrap(),
            r#"{"type":"subscribed","topic":"device:7"}"#
        );
        assert_eq!(
            serde_json::to_string(&WsServerMsg::Refused {
                topic: "x".into(),
                reason: RefusalReason::Forbidden
            })
            .unwrap(),
            r#"{"type":"refused","topic":"x","reason":"forbidden"}"#
        );
        assert_eq!(
            serde_json::from_str::<WsClientMsg>(r#"{"type":"subscribe","topic":"device:7"}"#)
                .unwrap(),
            WsClientMsg::Subscribe {
                topic: "device:7".into()
            }
        );
        assert_eq!(
            serde_json::from_str::<WsClientMsg>(r#"{"type":"unsubscribe","topic":"system"}"#)
                .unwrap(),
            WsClientMsg::Unsubscribe {
                topic: "system".into()
            }
        );
        assert!(
            serde_json::from_str::<WsClientMsg>(r#"{"type":"subscribe"}"#).is_err(),
            "sujet obligatoire"
        );
    }
}
