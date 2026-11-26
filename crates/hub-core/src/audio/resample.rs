pub const RATE: u32 = 48_000;

pub struct Resampler {
    step: f64,
    cutoff: f64,
    buf: Vec<f32>,
    pos: f64,
    passthrough: bool,
}

const HALF: usize = 16;

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let ratio = from as f64 / to as f64;
        Self {
            step: ratio,
            cutoff: (1.0 / ratio).min(1.0),
            buf: vec![0.0; HALF],
            pos: HALF as f64,
            passthrough: from == to,
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        self.buf.extend_from_slice(input);
        while self.pos + (HALF as f64) < self.buf.len() as f64 {
            let center = self.pos.floor() as isize;
            let mut acc = 0.0f64;
            for k in (center - HALF as isize + 1)..=(center + HALF as isize) {
                let x = self.pos - k as f64;
                let t = x / HALF as f64;
                let window = 0.5 + 0.5 * (std::f64::consts::PI * t).cos();
                let arg = std::f64::consts::PI * self.cutoff * x;
                let sinc = if arg.abs() < 1e-9 {
                    1.0
                } else {
                    arg.sin() / arg
                };
                acc += self.buf[k as usize] as f64 * sinc * window * self.cutoff;
            }
            out.push(acc as f32);
            self.pos += self.step;
        }
        let drop = (self.pos.floor() as usize).saturating_sub(HALF);
        if drop > 0 {
            self.buf.drain(..drop);
            self.pos -= drop as f64;
        }
    }

    pub fn flush(&mut self, out: &mut Vec<f32>) {
        if !self.passthrough {
            self.process(&[0.0; HALF + 1], out);
        }
    }
}
