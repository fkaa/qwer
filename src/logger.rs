use std::sync::{Arc, RwLock};

use slog::{o, Drain, OwnedKVList, Record};

use slog_term::Decorator;

pub(crate) fn initialize() -> (slog::Logger, ContextLogger) {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = MyFormat { decorator }.fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    let log = slog::Logger::root(drain, o!());
    let context = ContextLogger::new(&log);

    (log, context)
}

struct ContextSerializer<'a> {
    decorator: &'a mut dyn slog_term::RecordDecorator,
    done: bool,
}

impl<'a> slog::Serializer for ContextSerializer<'a> {
    fn emit_arguments(&mut self, _key: slog::Key, val: &std::fmt::Arguments) -> slog::Result {
        if !self.done {
            write!(self.decorator, "{:>10}", val)?;
            self.done = true;
        }

        Ok(())
    }
}

fn log_msg_header(
    fn_timestamp: &dyn slog_term::ThreadSafeTimestampFn<Output = std::io::Result<()>>,
    mut rd: &mut dyn slog_term::RecordDecorator,
    record: &slog::Record,
    values: &OwnedKVList,
    use_file_location: bool,
) -> std::io::Result<bool> {
    use std::io::Write;

    rd.start_timestamp()?;
    fn_timestamp(&mut rd)?;

    rd.start_whitespace()?;
    write!(rd, " ")?;

    {
        use slog::KV;

        values.serialize(
            record,
            &mut ContextSerializer {
                decorator: rd,
                done: false,
            },
        )?;
    }

    rd.start_whitespace()?;
    write!(rd, " ")?;

    rd.start_level()?;
    write!(rd, "{}", record.level().as_short_str())?;

    if use_file_location {
        rd.start_location()?;
        write!(
            rd,
            "[{}:{}:{}]",
            record.location().file,
            record.location().line,
            record.location().column
        )?;
    }

    rd.start_whitespace()?;
    write!(rd, " ")?;

    rd.start_msg()?;
    let mut count_rd = slog_term::CountingWriter::new(&mut rd);
    write!(count_rd, "{}", record.msg())?;
    Ok(count_rd.count() != 0)
}

struct MyFormat<D: Decorator> {
    decorator: D,
}

impl<D: Decorator> Drain for MyFormat<D> {
    type Ok = ();
    type Err = std::io::Error;

    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        self.decorator.with_record(record, values, |decorator| {
            let _comma_needed =
                log_msg_header(&slog_term::timestamp_local, decorator, record, values, true)?;

            {
                /* let mut serializer = Serializer::new(
                    decorator,
                    comma_needed,
                    self.use_original_order,
                );

                record.kv().serialize(record, &mut serializer)?;

                values.serialize(record, &mut serializer)?;

                serializer.finish()?;*/
            }

            decorator.start_whitespace()?;
            writeln!(decorator)?;

            decorator.flush()?;

            Ok(())
        })
    }
}

#[derive(Clone)]
pub struct ContextLogger {
    pub(crate) slog: slog::Logger,
    pub(crate) context: String,
    pub(crate) count: Arc<RwLock<u32>>,
}

impl std::ops::Deref for ContextLogger {
    type Target = slog::Logger;

    fn deref(&self) -> &Self::Target {
        &self.slog
    }
}

impl ContextLogger {
    pub fn new(slog: &slog::Logger) -> Self {
        let context = format!("1");
        let slog = slog.new(o!("context" => context.clone()));

        ContextLogger {
            slog,
            context,
            count: Arc::new(RwLock::new(1)),
        }
    }

    pub fn scope(&self) -> Self {
        let val = *self.count.read().unwrap();
        (*self.count.write().unwrap()) = val + 1;

        let context = format!("{}.{}", self.context, val);
        let slog = self.slog.new(o!("context" => context.clone()));

        ContextLogger {
            slog,
            context,
            count: Arc::new(RwLock::new(1)),
        }
    }
}

/*tokio::task_local! {
    pub(crate) static CONTEXT: Context
}

#[derive(Clone)]
pub(crate) struct Context {
    pub(crate) context: String,
    pub(crate) count: Arc<RwLock<u32>>,
}

impl Context {
    pub(crate) fn new(prev: Option<Context>) -> Self {
        let (context, count) = if let Some(cxt) = prev {
            let val = *cxt.count.read().unwrap();
            (*cxt.count.write().unwrap()) = val + 1;
            (format!("{}.{}", cxt.context, val), Arc::new(RwLock::new(0)))
        } else {
            (format!("1"), Arc::new(RwLock::new(0)))
        };

        Context {
            context,
            count
        }
    }
}

#[macro_export]
macro_rules! scope {
    ($do:expr) => {
        {
            let f = |c: Option<&crate::logger::Context>| crate::logger::CONTEXT.sync_scope(crate::logger::Context::new(c), $do);
            let r = crate::logger::CONTEXT.try_with(|c| {
                f(Some(c))
            });
            match r {
                Ok(result) => result,
                Err(_) => f(None)
            }
        }
    }
}

#[macro_export]
macro_rules! async_scope {
    ($do:expr) => {
        {
            let c = crate::logger::CONTEXT.try_with(|c| {
                c.clone()
            }).ok();
            crate::logger::CONTEXT.scope(crate::logger::Context::new(c), $do)
        }
    }
}

pub(crate) use async_scope;

#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => {
        if !crate::logger::CONTEXT.try_with(|c| log::debug!(target: &c.context, $($arg)+)).is_ok() {
            log::debug!($($arg)+)
        }
    }
}

pub(crate) use debug;

#[macro_export]
macro_rules! warn_ {
    ($($arg:tt)+) => {
        if !crate::logger::CONTEXT.try_with(|c| log::warn!(target: &c.context, $($arg)+)).is_ok() {
            log::warn!($($arg)+)
        }
    }
}

pub(crate) use warn_ as warn;

macro_rules! info {
    ($($arg:tt)+) => {
        if !crate::logger::CONTEXT.try_with(|c| log::info!(target: &c.context, $($arg)+)).is_ok() {
            log::info!($($arg)+)
        }
    }
}

pub(crate) use info;

#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => {
        if !crate::logger::CONTEXT.try_with(|c| log::error!(target: &c.context, $($arg)+)).is_ok() {
            log::error!($($arg)+)
        }
    }
}

pub(crate) use error;
*/
