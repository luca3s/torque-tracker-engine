#[cfg(test)]
mod tests {
    use simple_left_right::writer::Writer;
    use std::time::Duration;

    include!("utilities.rs");

    #[test]
    fn send_writer() {
        let mut writer = Writer::new(0);
        let (send, rcv) = std::sync::mpsc::channel::<Writer<i32, CounterAddOp>>();
        std::thread::spawn(move || {
            let writer = rcv.recv().unwrap();
            assert_eq!(*writer, 1);
        });
        writer.try_lock().unwrap().apply_op(CounterAddOp(1));
        send.send(writer).unwrap();
    }

    #[test]
    fn write_guard_drop() {
        let mut writer = Writer::new(0);
        let mut reader = writer.build_reader().unwrap();
        assert_eq!(*reader.lock(), 0);
        let mut write_lock = writer.lock();
        assert_eq!(*write_lock, 0);
        write_lock.apply_op(CounterAddOp(2));
        assert_eq!(*write_lock, 2);
        // drop(write_lock);
        let write_lock = writer.lock();
        assert_eq!(*write_lock, 2);
        write_lock.swap();
        assert_eq!(*reader.lock(), 2);
        assert_eq!(*writer.lock(), 2);
    }

    #[test]
    fn write_try_lock() {
        let mut writer = Writer::new(0);
        let mut lock = writer.try_lock().unwrap();
        lock.apply_op(CounterAddOp(1));
        assert_eq!(*lock, 1);
        assert_eq!(*writer.try_lock().unwrap(), 1);
    }

    #[test]
    fn writer_as_ref() {
        let mut writer = Writer::new(0);
        assert_eq!(*writer, 0);

        writer.try_lock().unwrap().apply_op(CounterAddOp(3));
        assert_eq!(*writer, 3);
    }

    #[test]
    fn single_thread() {
        let mut writer = Writer::new(0);
        let mut reader = writer.build_reader().unwrap();

        let read_lock = reader.lock();
        assert_eq!(*read_lock, 0);

        let mut write_lock = writer.try_lock().unwrap();
        write_lock.apply_op(CounterAddOp(1));

        assert_eq!(*read_lock, 0);

        drop(read_lock);

        let read_lock = reader.lock();
        assert_eq!(*read_lock, 0);

        write_lock.swap();
        // buffers got swapped, but a read_lock is being held. Therefore, a new write_lock isn't possible
        assert!(writer.try_lock().is_none());
        drop(read_lock);

        let read_lock = reader.lock();
        // read_lock was dropped and newly acquired: New values can be read and a write lock can be acquired
        assert_eq!(*read_lock, 1);
        let write_lock = writer.try_lock().unwrap();
        write_lock.swap();

        drop(read_lock);

        let read_lock = reader.lock();
        assert_eq!(*read_lock, 1);
    }

    #[test]
    fn block() {
        let mut writer = Writer::new(0);
        let mut reader = writer.build_reader().unwrap();
        writer.lock().apply_op(CounterAddOp(2));
        std::thread::spawn(move || {
            let lock = reader.lock();
            assert_eq!(*lock, 0);
            std::thread::sleep(Duration::from_secs(2));
            drop(lock);
            assert_eq!(*reader.lock(), 2);
        });
        std::thread::sleep(Duration::from_secs(1));
        writer.lock().swap();
        // blocks until the spawned thread drops the read_lock
        let write_lock = writer.lock();
        assert_eq!(*write_lock, 2);
    }

    #[test]
    fn multi_thread() {
        let mut writer = Writer::new(0);
        let mut reader = writer.build_reader().unwrap();
        let mut write_lock = writer.try_lock().unwrap();

        let thread = std::thread::spawn(move || {
            let read_lock = reader.lock();
            assert_eq!(*read_lock, 0);
            std::thread::sleep(Duration::from_secs(3));
            assert_eq!(*read_lock, 0);
            drop(read_lock);
        });
        // make sure the spawned thread gets the old value, not the new
        std::thread::sleep(Duration::from_secs(1));
        write_lock.apply_op(CounterAddOp(1));
        write_lock.swap();
        assert!(writer.try_lock().is_none());
        thread.join().unwrap();
        let _write_lock = writer.try_lock().unwrap();
    }

    #[test]
    fn double_reader() {
        let mut writer: Writer<i32, CounterAddOp> = Writer::new(0);
        let _reader = writer.build_reader().unwrap();
        assert!(writer.build_reader().is_none());
    }

    #[test]
    // leaks so miri unhappy
    #[cfg_attr(miri, ignore)]
    fn reader_leak() {
        let mut writer: Writer<i32, CounterAddOp> = Writer::new(0);
        let reader = writer.build_reader().unwrap();
        drop(reader);
        let reader = writer.build_reader().unwrap();
        std::mem::forget(reader);
        assert!(writer.build_reader().is_none());
    }

    #[test]
    fn reader_rebuild() {
        let mut writer: Writer<i32, CounterAddOp> = Writer::default();
        let reader = writer.build_reader().unwrap();
        drop(reader);
        let _reader = writer.build_reader().unwrap();
    }

    #[test]
    fn default_init() {
        let writer: Writer<i32, CounterAddOp> = Writer::default();
        assert_eq!(*writer, i32::default());
    }

    // #[test]
    // fn box_init() {
    //     let value = Box::new(2);
    //     let writer = Writer::new_from_box(value);
    //     assert_eq!(*writer.as_ref(), 2);
    // }
}
