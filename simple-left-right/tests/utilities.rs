use simple_left_right::writer::Absorb;

#[cfg(test)]
#[derive(Clone)]
pub struct CounterAddOp(i32);

#[cfg(test)]
impl Absorb<CounterAddOp> for i32 {
    fn absorb(&mut self, operation: CounterAddOp) {
        *self += operation.0;
    }
}
