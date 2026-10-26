use std::{
    ffi::{CStr, CString},
    rc::Rc,
};

use crate::{
    engine::host_call,
    ffi::{c_str, guard},
    host::Host,
    Engine,
};

/// Words of the console command being run, copied out of the engine so that
/// they stay valid even if the handler runs other commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandArgs {
    words: Vec<CString>,
    rest:  CString,
}

impl CommandArgs {
    /// Number of words, including the command name.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether there is no command at all.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Word `index`; word 0 is the command name.
    pub fn get(&self, index: usize) -> Option<&CStr> {
        self.words.get(index).map(CString::as_c_str)
    }

    /// Every word, starting with the command name.
    pub fn iter(&self) -> impl Iterator<Item = &CStr> {
        self.words.iter().map(CString::as_c_str)
    }

    /// Everything after the command name, as typed.
    pub fn rest(&self) -> &CStr {
        &self.rest
    }
}

/// A server command registered by this plugin.
pub(crate) struct Command {
    name:    &'static CStr,
    handler: Rc<dyn Fn(&CommandArgs)>,
}

impl Engine {
    /// The words of the command the engine is running right now, such as a
    /// client command inside the `client_command` hook.
    pub fn command_args(&self) -> CommandArgs {
        // SAFETY: reads the word count of the engine's current command.
        let count = unsafe { host_call!(self.functions, pfnCmd_Argc()) };
        let word = |index| {
            // SAFETY: the engine returns a word of its current command, which
            // is copied before anything else runs.
            unsafe { c_str(host_call!(self.functions, pfnCmd_Argv(index))) }
                .unwrap_or_default()
                .to_owned()
        };
        // SAFETY: the engine returns the text after the command name, which
        // is copied before anything else runs.
        let rest = unsafe { c_str(host_call!(self.functions, pfnCmd_Args())) }
            .unwrap_or_default()
            .to_owned();
        CommandArgs {
            words: (0..count).map(word).collect(),
            rest,
        }
    }

    /// Adds a server console command that runs `handler`. Registering a name
    /// again replaces its handler. Names are matched without case, as the
    /// engine and metamod do.
    pub fn register_command(&self, name: &'static CStr, handler: impl Fn(&CommandArgs) + 'static) {
        let handler: Rc<dyn Fn(&CommandArgs)> = Rc::new(handler);
        let mut commands = self.commands.borrow_mut();
        if let Some(command) = commands
            .iter_mut()
            .find(|command| same_name(command.name, name))
        {
            command.handler = handler;
            return;
        }
        commands.push(Command { name, handler });
        drop(commands);
        // SAFETY: the name is static and `run_command` is a valid callback
        // for as long as this plugin is loaded; metamod disables the command
        // when the plugin unloads.
        unsafe {
            host_call!(
                self.functions,
                pfnAddServerCommand(name.as_ptr().cast_mut(), Some(run_command))
            )
        }
    }
}

fn same_name(left: &CStr, right: &CStr) -> bool {
    left.to_bytes().eq_ignore_ascii_case(right.to_bytes())
}

/// The one callback every registered command shares: like metamod's own
/// dispatcher, it finds the handler by the command name in `argv[0]`.
unsafe extern "C" fn run_command() {
    guard("console command", || {
        let Some(engine) = Host::engine() else {
            return log::error!("console command ran before the engine was connected");
        };
        let args = engine.command_args();
        let name = args.get(0).unwrap_or_default();
        // Clone the handler out so that it can register commands itself.
        let handler = engine
            .commands
            .borrow()
            .iter()
            .find(|command| same_name(command.name, name))
            .map(|command| Rc::clone(&command.handler));
        match handler {
            Some(handler) => handler(&args),
            None => log::warn!("no handler for console command {name:?}"),
        }
    });
}
