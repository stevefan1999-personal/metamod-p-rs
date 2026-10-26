use std::{
    ffi::c_int,
    fmt,
    hash::{Hash, Hasher},
    ptr::{addr_of, addr_of_mut, NonNull},
};

use crate::{
    engine::host_call,
    sys::{edict_t, entvars_t, Vector, FL_CLIENT},
    Engine,
};

/// An entity slot, checked against the engine's own bookkeeping.
///
/// A handle remembers the slot's serial number, which the engine bumps each
/// time it frees the slot, so [`Edict::is_valid`] tells a live entity from a
/// slot that was freed or reused since. Hooks hand out fresh handles; check
/// stored ones with `is_valid` before use and drop them when the map changes.
#[derive(Clone, Copy)]
pub struct Edict {
    raw:    NonNull<edict_t>,
    serial: c_int,
    engine: &'static Engine,
}

impl Edict {
    /// Wraps an engine edict pointer. Returns `None` for a null pointer or a
    /// free slot.
    ///
    /// # Safety
    ///
    /// A non-null `raw` must point into the engine's edict array.
    pub unsafe fn from_raw(raw: *mut edict_t, engine: &'static Engine) -> Option<Self> {
        let raw = NonNull::new(raw)?;
        // SAFETY: the caller guarantees that `raw` points to an engine edict.
        let (free, serial) = unsafe {
            (
                addr_of!((*raw.as_ptr()).free).read(),
                addr_of!((*raw.as_ptr()).serialnumber).read(),
            )
        };
        (free == 0).then_some(Self {
            raw,
            serial,
            engine,
        })
    }

    /// The raw edict pointer, for calls the safe API does not wrap.
    pub fn as_ptr(&self) -> *mut edict_t {
        self.raw.as_ptr()
    }

    /// Whether the slot still holds the entity this handle was made for.
    pub fn is_valid(&self) -> bool {
        // SAFETY: see `Edict::vars`; both fields are plain integers.
        let (free, serial) = unsafe {
            (
                addr_of!((*self.raw.as_ptr()).free).read(),
                addr_of!((*self.raw.as_ptr()).serialnumber).read(),
            )
        };
        free == 0 && serial == self.serial
    }

    /// Entity index: 0 is the world, `1..=max_clients` are player slots.
    pub fn index(&self) -> i32 {
        // SAFETY: `raw` points into the edict array, which is all that
        // IndexOfEdict needs to compute the slot number.
        unsafe { host_call!(self.engine.functions, pfnIndexOfEdict(self.raw.as_ptr())) }
    }

    /// Whether a client is in the game in this slot (`FL_CLIENT`).
    pub fn is_client(&self) -> bool {
        self.flags() & FL_CLIENT as i32 != 0
    }

    // SAFETY (for every accessor): `raw` points into the engine's edict
    // array, which stays allocated while the server runs. Accessors read and
    // write single plain-old-data fields through raw pointers and never make
    // references, so a stale handle sees another entity's data, not freed
    // memory.
    fn vars(&self) -> *mut entvars_t {
        // SAFETY: only computes a field address, see above.
        unsafe { addr_of_mut!((*self.raw.as_ptr()).v) }
    }
}

impl PartialEq for Edict {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw && self.serial == other.serial
    }
}

impl Eq for Edict {}

impl Hash for Edict {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.raw, self.serial).hash(state);
    }
}

impl fmt::Debug for Edict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Edict")
            .field("raw", &self.raw)
            .field("serial", &self.serial)
            .finish()
    }
}

/// Reads an `entvars_t` field as the type its accessor returns.
trait FromField<Raw> {
    fn from_field(raw: Raw, engine: &'static Engine) -> Self;
}

/// Writes an accessor value back as the `entvars_t` field type.
trait IntoField<Raw> {
    fn into_field(self) -> Raw;
}

impl FromField<f32> for f32 {
    fn from_field(raw: f32, _: &'static Engine) -> Self {
        raw
    }
}

impl IntoField<f32> for f32 {
    fn into_field(self) -> f32 {
        self
    }
}

impl FromField<c_int> for i32 {
    fn from_field(raw: c_int, _: &'static Engine) -> Self {
        raw
    }
}

impl IntoField<c_int> for i32 {
    fn into_field(self) -> c_int {
        self
    }
}

impl FromField<Vector> for [f32; 3] {
    fn from_field(raw: Vector, _: &'static Engine) -> Self {
        [raw.x, raw.y, raw.z]
    }
}

impl IntoField<Vector> for [f32; 3] {
    fn into_field(self) -> Vector {
        let [x, y, z] = self;
        Vector { x, y, z }
    }
}

/// `string_t` fields are offsets into the engine's string table.
impl FromField<c_int> for Option<String> {
    fn from_field(raw: c_int, engine: &'static Engine) -> Self {
        engine.string(raw)
    }
}

impl FromField<*mut edict_t> for Option<Edict> {
    fn from_field(raw: *mut edict_t, engine: &'static Engine) -> Self {
        // SAFETY: entity fields hold null or pointers into the edict array.
        unsafe { Edict::from_raw(raw, engine) }
    }
}

/// Generates a getter (and an optional setter) per `entvars_t` field.
macro_rules! entvars_fields {
    ($($(#[$doc:meta])* $getter:ident $(/ $setter:ident)? = $field:ident: $value:ty;)*) => {
        impl Edict {
            $(
                $(#[$doc])*
                pub fn $getter(&self) -> $value {
                    // SAFETY: see `Edict::vars`.
                    let raw = unsafe { addr_of!((*self.vars()).$field).read() };
                    FromField::from_field(raw, self.engine)
                }

                $(
                    #[doc = concat!("Sets [`Edict::", stringify!($getter), "`].")]
                    pub fn $setter(&self, value: $value) {
                        // SAFETY: see `Edict::vars`.
                        unsafe { addr_of_mut!((*self.vars()).$field).write(value.into_field()) }
                    }
                )?
            )*
        }
    };
}

entvars_fields! {
    /// Entity class, such as `player` or `weapon_crowbar`.
    class_name = classname: Option<String>;
    /// Player name.
    net_name = netname: Option<String>;
    /// Model path.
    model = model: Option<String>;
    /// First-person weapon model of a player.
    view_model = viewmodel: Option<String>;
    /// Third-person weapon model of a player.
    weapon_model = weaponmodel: Option<String>;
    /// Name that other entities target this one by.
    target_name = targetname: Option<String>;
    /// Name of the entity that this one triggers.
    target = target: Option<String>;
    /// Owner, such as the player who fired a projectile.
    owner = owner: Option<Edict>;
    /// Position. Writing it does not relink the entity; use the engine's
    /// `SetOrigin` for teleports.
    origin / set_origin = origin: [f32; 3];
    /// Velocity in units per second.
    velocity / set_velocity = velocity: [f32; 3];
    /// Orientation (pitch, yaw, roll) in degrees.
    angles / set_angles = angles: [f32; 3];
    /// View angles of a player.
    view_angles / set_view_angles = v_angle: [f32; 3];
    /// View punch (recoil) of a player.
    punch_angles / set_punch_angles = punchangle: [f32; 3];
    /// Health points.
    health / set_health = health: f32;
    /// Health limit.
    max_health / set_max_health = max_health: f32;
    /// Armor points.
    armor / set_armor = armorvalue: f32;
    /// Score of a player.
    frags / set_frags = frags: f32;
    /// Damage mode (`DAMAGE_NO`, `DAMAGE_YES`, `DAMAGE_AIM`).
    take_damage / set_take_damage = takedamage: f32;
    /// Movement speed limit of a player.
    max_speed / set_max_speed = maxspeed: f32;
    /// Gravity factor (1 is normal).
    gravity / set_gravity = gravity: f32;
    /// Field of view of a player.
    field_of_view / set_field_of_view = fov: f32;
    /// Render amount (alpha) in 0..=255.
    render_amount / set_render_amount = renderamt: f32;
    /// Render color in 0..=255 per channel.
    render_color / set_render_color = rendercolor: [f32; 3];
    /// Render mode (`kRenderNormal` and so on).
    render_mode / set_render_mode = rendermode: i32;
    /// Render effect (`kRenderFxNone` and so on).
    render_fx / set_render_fx = renderfx: i32;
    /// Effect flags (`EF_*`).
    effects / set_effects = effects: i32;
    /// Entity flags (`FL_*`).
    flags / set_flags = flags: i32;
    /// Team number.
    team / set_team = team: i32;
    /// Movement type (`MOVETYPE_*`).
    move_type / set_move_type = movetype: i32;
    /// Collision type (`SOLID_*`).
    solid / set_solid = solid: i32;
    /// Death state (`DEAD_*`).
    dead_flag / set_dead_flag = deadflag: i32;
    /// Weapon bits of a player.
    weapons / set_weapons = weapons: i32;
    /// Buttons a player holds (`IN_*`).
    buttons / set_buttons = button: i32;
    /// Index of the precached model.
    model_index = modelindex: i32;
}

impl Engine {
    /// The entity at `index`, or `None` for an empty slot. 0 is the world
    /// and `1..=max_clients` are player slots.
    pub fn edict(&'static self, index: i32) -> Option<Edict> {
        // SAFETY: PEntityOfEntIndex returns null or a pointer into the edict
        // array.
        unsafe {
            Edict::from_raw(
                host_call!(self.functions, pfnPEntityOfEntIndex(index)),
                self,
            )
        }
    }

    /// Player slots that hold a client in the game.
    pub fn players(&'static self) -> impl Iterator<Item = Edict> {
        (1..=self.max_clients())
            .filter_map(|index| self.edict(index))
            .filter(Edict::is_client)
    }
}
