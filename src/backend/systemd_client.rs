//! Talks to the user's systemd instance over D-Bus, instead of shelling out to `systemctl`.
//!
//! Two parts:
//! - `SystemdClient`: thin async wrapper over the `org.freedesktop.systemd1.Manager` calls we use.
//! - `watch_events`: a background task that listens for systemd signals and keeps the
//!   sysd sessions in `DaemonManager` in sync with what systemd reports, so a plain
//!   `systemctl --user disable foo` or `systemctl --user stop foo` is reflected in the daemon.

use tracing::{debug, trace, warn};
use std::sync::Arc;

use color_eyre::eyre::{Result, WrapErr, bail};
use futures::StreamExt;
use tokio::sync::Mutex;
use zbus_systemd::systemd1::{ManagerProxy, UnitProxy};
use zbus_systemd::zbus::{self, Connection, MatchRule, MessageStream, fdo::PropertiesChanged};

use crate::backend::daemon_manager::DaemonManager;
use crate::backend::session_manager::UnitName;

/// Job mode for start/stop/restart. "replace" is what `systemctl` uses by default.
const JOB_MODE: &str = "replace";

const UNIT_PATH_PREFIX: &str = "/org/freedesktop/systemd1/unit/";
const UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";

#[derive(Clone)]
pub struct SystemdClient {
    manager: ManagerProxy<'static>,
}

impl SystemdClient {
    /// Connects to the user's session bus (the bus the `systemd --user` instance lives on).
    pub async fn connect() -> Result<Self> {
        let connection = Connection::session()
            .await
            .wrap_err("Failed to connect to the D-Bus session bus")?;
        let manager = ManagerProxy::new(&connection)
            .await
            .wrap_err("Failed to create systemd manager proxy")?;

        Ok(Self { manager })
    }

    pub async fn start(&self, unit: &UnitName) -> Result<()> {
        debug!(unit = %service_name(unit), "systemd: start");
        self.manager
            .start_unit(service_name(unit), JOB_MODE.into())
            .await
            .wrap_err_with(|| format!("Failed to start {}", service_name(unit)))?;
        Ok(())
    }

    pub async fn stop(&self, unit: &UnitName) -> Result<()> {
        debug!(unit = %service_name(unit), "systemd: stop");
        self.manager
            .stop_unit(service_name(unit), JOB_MODE.into())
            .await
            .wrap_err_with(|| format!("Failed to stop {}", service_name(unit)))?;
        Ok(())
    }

    pub async fn restart(&self, unit: &UnitName) -> Result<()> {
        debug!(unit = %service_name(unit), "systemd: restart");
        self.manager
            .restart_unit(service_name(unit), JOB_MODE.into())
            .await
            .wrap_err_with(|| format!("Failed to restart {}", service_name(unit)))?;
        Ok(())
    }

    /// Equivalent of `systemctl --user enable`: creates the install symlinks, then reloads.
    pub async fn enable(&self, unit: &UnitName) -> Result<()> {
        debug!(unit = %service_name(unit), "systemd: enable");
        self.manager
            .enable_unit_files(vec![service_name(unit)], false, true)
            .await
            .wrap_err_with(|| format!("Failed to enable {}", service_name(unit)))?;
        self.reload().await
    }

    /// Equivalent of `systemctl --user disable`: removes the install symlinks, then reloads.
    pub async fn disable(&self, unit: &UnitName) -> Result<()> {
        debug!(unit = %service_name(unit), "systemd: disable");
        self.manager
            .disable_unit_files(vec![service_name(unit)], false)
            .await
            .wrap_err_with(|| format!("Failed to disable {}", service_name(unit)))?;
        self.reload().await
    }

    /// Equivalent of `systemctl --user daemon-reload`.
    pub async fn reload(&self) -> Result<()> {
        debug!("systemd: daemon-reload");
        self.manager
            .reload()
            .await
            .wrap_err("Failed to reload systemd")?;
        Ok(())
    }

    /// Asks systemd whether a new `<unit>.service` may be created under this name.
    ///
    /// Errors if systemd rejects the name, or already knows a unit by it: a unit file
    /// anywhere in its search path (packaged ones like `/usr/lib/systemd/user/...`
    /// included), a masked unit, or a running transient one. Writing our own unit file
    /// over such a name would replace or shadow that unit.
    ///
    /// Uses `LoadUnit`, which validates the name with systemd's own rules and reports
    /// `LoadState = not-found` when there is nothing by that name.
    pub async fn ensure_unit_name_free(&self, unit: &UnitName) -> Result<()> {
        let name = service_name(unit);
        let path = self
            .manager
            .load_unit(name.clone())
            .await
            .wrap_err_with(|| format!("systemd does not accept the name {name}"))?;

        let proxy = UnitProxy::new(self.manager.inner().connection(), path)
            .await
            .wrap_err("Failed to create systemd unit proxy")?;
        let load_state = proxy
            .load_state()
            .await
            .wrap_err_with(|| format!("Failed to get the load state of {name}"))?;
        trace!(unit = %name, %load_state, "Checked unit name");

        if load_state == "not-found" {
            return Ok(());
        }
        let fragment = proxy.fragment_path().await.unwrap_or_default();
        if fragment.is_empty() {
            bail!("systemd already has a unit named {name} ({load_state})");
        }
        bail!("systemd already has a unit named {name} ({load_state}, from {fragment})");
    }

    /// True if the unit file state is `enabled` (or a variant like `enabled-runtime`).
    pub async fn is_enabled(&self, unit: &UnitName) -> Result<bool> {
        let state = self
            .manager
            .get_unit_file_state(service_name(unit))
            .await
            .wrap_err_with(|| format!("Failed to get unit file state of {}", service_name(unit)))?;
        Ok(state.starts_with("enabled"))
    }
}

fn service_name(unit: &UnitName) -> String {
    format!("{unit}.service")
}

// ---------------------------------------------------------------------------
// Event watching
// ---------------------------------------------------------------------------

/// Subscribes to systemd signals and applies them to the daemon state until the bus goes away.
///
/// - `UnitFilesChanged` / `Reloading`: an enable/disable/daemon-reload happened somewhere, so
///   re-read the enabled state of every sysd session.
/// - `PropertiesChanged` on a unit's `ActiveState`: if one of our units went `inactive` or
///   `failed`, tear down its windows. Starting is not handled here, because `ExecStart`
///   already tells the daemon via `__internal-systemd-start`.
///
/// Signals arrive on their own connection. zbus stops reading a connection while any of
/// its signal streams is full, so sharing one with method calls lets a burst of unit
/// signals block the replies we are waiting for (including the ones this loop awaits).
pub async fn watch_events(client: SystemdClient, manager: Arc<Mutex<DaemonManager>>) -> Result<()> {
    let signal_connection = Connection::session()
        .await
        .wrap_err("Failed to open D-Bus connection for systemd signals")?;
    let signals = ManagerProxy::new(&signal_connection).await?;

    // systemd only broadcasts unit/job signals while at least one client is subscribed.
    signals
        .subscribe()
        .await
        .wrap_err("Failed to subscribe to systemd signals")?;

    let mut unit_files_changed = signals.receive_unit_files_changed().await?;
    let mut reloading = signals.receive_reloading().await?;

    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path_namespace("/org/freedesktop/systemd1/unit")?
        .arg(0, UNIT_INTERFACE)?
        .build();
    // Every unit's changes match this, so leave room for bursts (e.g. many units starting at once).
    let mut unit_properties =
        MessageStream::for_match_rule(rule, &signal_connection, Some(1024)).await?;

    debug!("Watching systemd signals");

    // Units registered from disk at startup do not know their enabled state yet.
    refresh_enabled(&client, &manager).await;

    loop {
        tokio::select! {
            Some(_) = unit_files_changed.next() => refresh_enabled(&client, &manager).await,
            Some(signal) = reloading.next() => {
                // Fires with `true` when a reload starts and `false` when it is done.
                if matches!(signal.args().map(|args| args.active), Ok(false)) {
                    refresh_enabled(&client, &manager).await;
                }
            }
            Some(message) = unit_properties.next() => {
                if let Ok(message) = message {
                    handle_properties_changed(message, &manager).await;
                }
            }
            else => break,
        }
    }

    warn!("systemd event stream ended; external systemctl changes are no longer tracked");
    Ok(())
}

async fn refresh_enabled(client: &SystemdClient, manager: &Arc<Mutex<DaemonManager>>) {
    // Do not hold the lock across D-Bus calls.
    let units = manager.lock().await.sysd_units();

    for unit in units {
        match client.is_enabled(&unit).await {
            Ok(enabled) => {
                trace!(unit = %unit, enabled, "Refreshed enabled state");
                manager.lock().await.set_sysd_enabled(&unit, enabled)
            }
            Err(e) => warn!("{e:#}"),
        }
    }
}

async fn handle_properties_changed(message: zbus::Message, manager: &Arc<Mutex<DaemonManager>>) {
    let Some(unit) = message
        .header()
        .path()
        .and_then(|path| unit_from_object_path(path.as_str()))
    else {
        return;
    };
    let Some(signal) = PropertiesChanged::from_message(message) else {
        return;
    };
    let Ok(args) = signal.args() else {
        return;
    };
    let Some(active_state) = args
        .changed_properties()
        .get("ActiveState")
        .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
    else {
        return;
    };

    trace!(unit = %unit, %active_state, "Unit state changed");

    if matches!(active_state.as_str(), "inactive" | "failed") {
        manager.lock().await.handle_unit_deactivated(&unit);
    }
}

/// Maps `/org/freedesktop/systemd1/unit/foo_2eservice` back to the unit `foo`.
/// Returns `None` for anything that is not a `.service` unit.
pub fn unit_from_object_path(path: &str) -> Option<UnitName> {
    let label = path.strip_prefix(UNIT_PATH_PREFIX)?;
    let name = unescape_bus_label(label)?;
    let name = name.strip_suffix(".service")?;
    UnitName::new(name).ok()
}

/// Reverses systemd's `bus_label_escape`: every byte outside `[A-Za-z0-9]` is written as `_xx`.
pub fn unescape_bus_label(label: &str) -> Option<String> {
    let bytes = label.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'_' {
            let hex = label.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }

    String::from_utf8(out).ok()
}
