use smithay::backend::libei::{EisContextSource, EisListenerSource};
use smithay::reexports::calloop;

fn f<State>(handle: calloop::LoopHandle<'static, State>) {
    let source: EisListenerSource = todo!();
    handle
        .insert_source(source, |context, _, _| {
            EisContextSource::new(context);
        })
        .unwrap();
}
