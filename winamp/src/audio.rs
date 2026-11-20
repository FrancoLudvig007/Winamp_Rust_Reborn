//! Motor de áudio: decodificação (rodio), equalizador de 10 bandas (biquads),
//! balanço, contagem de posição e captura de amostras para o analisador de espectro.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

pub const BANDS: usize = 10;
pub const EQ_FREQS: [f32; BANDS] = [
    70.0, 180.0, 320.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0,
];
const SCOPE_MAX: usize = 4096;

/// Estado compartilhado entre a thread de UI e a thread de áudio (sem locks no caminho quente).
pub struct Shared {
    eq_on: AtomicBool,
    /// Décimos de dB. Índice 0 = preamp, 1..=10 = bandas.
    eq_gain: [AtomicI32; BANDS + 1],
    eq_ver: AtomicU32,
    /// Balanço em centésimos (-100 = esquerda, 100 = direita).
    balance: AtomicI32,
    /// Quadros (frames) consumidos desde o último `load`.
    frames: AtomicU64,
    scope: Mutex<Vec<f32>>,
}

impl Shared {
    fn new() -> Self {
        Shared {
            eq_on: AtomicBool::new(false),
            eq_gain: std::array::from_fn(|_| AtomicI32::new(0)),
            eq_ver: AtomicU32::new(1),
            balance: AtomicI32::new(0),
            frames: AtomicU64::new(0),
            scope: Mutex::new(Vec::with_capacity(SCOPE_MAX + 512)),
        }
    }

    pub fn set_eq(&self, on: bool, preamp_db: f32, bands_db: &[f32; BANDS]) {
        self.eq_on.store(on, Relaxed);
        self.eq_gain[0].store((preamp_db * 10.0).round() as i32, Relaxed);
        for (i, g) in bands_db.iter().enumerate() {
            self.eq_gain[i + 1].store((g * 10.0).round() as i32, Relaxed);
        }
        self.eq_ver.fetch_add(1, Relaxed);
    }

    pub fn set_balance(&self, b: f32) {
        self.balance.store((b.clamp(-1.0, 1.0) * 100.0).round() as i32, Relaxed);
    }
}

#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn identity() -> Self {
        Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }

    /// Filtro "peaking EQ" (RBJ cookbook).
    fn set_peaking(&mut self, fs: f32, f0: f32, gain_db: f32, q: f32) {
        if gain_db.abs() < 0.05 || f0 >= fs * 0.47 {
            self.b0 = 1.0;
            self.b1 = 0.0;
            self.b2 = 0.0;
            self.a1 = 0.0;
            self.a2 = 0.0;
            return;
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * f0 / fs;
        let alpha = w0.sin() / (2.0 * q);
        let c = w0.cos();
        let a0 = 1.0 + alpha / a;
        self.b0 = (1.0 + alpha * a) / a0;
        self.b1 = (-2.0 * c) / a0;
        self.b2 = (1.0 - alpha * a) / a0;
        self.a1 = (-2.0 * c) / a0;
        self.a2 = (1.0 - alpha / a) / a0;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Adaptador de `Source`: aplica EQ + balanço e alimenta o analisador de espectro.
struct Tap<S: Source<Item = i16>> {
    inner: S,
    sh: Arc<Shared>,
    ch: u16,
    rate: u32,
    chan: usize,
    filt: Vec<[Biquad; BANDS]>,
    eq_on: bool,
    preamp: f32,
    bal: f32,
    ver: u32,
    tick: u32,
    frames_local: u64,
    buf: Vec<f32>,
}

impl<S: Source<Item = i16>> Tap<S> {
    fn new(inner: S, sh: Arc<Shared>) -> Self {
        let ch = inner.channels().max(1);
        let rate = inner.sample_rate().max(1);
        Tap {
            inner,
            sh,
            ch,
            rate,
            chan: 0,
            filt: vec![[Biquad::identity(); BANDS]; ch as usize],
            eq_on: false,
            preamp: 1.0,
            bal: 0.0,
            ver: 0,
            tick: 0,
            frames_local: 0,
            buf: Vec::with_capacity(512),
        }
    }

    fn refresh(&mut self) {
        self.eq_on = self.sh.eq_on.load(Relaxed);
        self.bal = self.sh.balance.load(Relaxed) as f32 / 100.0;
        if self.frames_local > 0 {
            self.sh.frames.fetch_add(self.frames_local, Relaxed);
            self.frames_local = 0;
        }
        let v = self.sh.eq_ver.load(Relaxed);
        if v != self.ver {
            self.ver = v;
            self.preamp = 10f32.powf(self.sh.eq_gain[0].load(Relaxed) as f32 / 200.0);
            for f in self.filt.iter_mut() {
                for (b, bq) in f.iter_mut().enumerate() {
                    let g = self.sh.eq_gain[b + 1].load(Relaxed) as f32 / 10.0;
                    bq.set_peaking(self.rate as f32, EQ_FREQS[b], g, 1.4);
                }
            }
        }
    }
}

impl<S: Source<Item = i16>> Iterator for Tap<S> {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let s = self.inner.next()? as f32 / 32768.0;
        if self.tick & 511 == 0 {
            self.refresh();
        }
        self.tick = self.tick.wrapping_add(1);
        let c = self.chan;
        self.chan = (c + 1) % self.ch as usize;

        let mut y = s;
        if self.eq_on {
            y *= self.preamp;
            for bq in self.filt[c].iter_mut() {
                y = bq.process(y);
            }
        }
        if self.ch == 2 {
            y *= if c == 0 { 1.0 - self.bal.max(0.0) } else { 1.0 + self.bal.min(0.0) };
        }
        if c == 0 {
            self.frames_local += 1;
            self.buf.push(y);
            if self.buf.len() >= 512 {
                if let Ok(mut g) = self.sh.scope.try_lock() {
                    g.extend_from_slice(&self.buf);
                    if g.len() > SCOPE_MAX {
                        let excess = g.len() - SCOPE_MAX;
                        g.drain(..excess);
                    }
                }
                self.buf.clear();
            }
        }
        Some((y.clamp(-1.0, 1.0) * 32767.0) as i16)
    }
}

impl<S: Source<Item = i16>> Source for Tap<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.ch
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

pub struct Engine {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Option<Sink>,
    pub sh: Arc<Shared>,
    volume: f32,
    base: f64,
    paused: bool,
    path: Option<PathBuf>,
    pub duration: Option<f64>,
    pub sample_rate: u32,
    pub channels: u16,
    pub kbps: Option<u32>,
}

impl Engine {
    pub fn new() -> Result<Self, String> {
        let (stream, handle) = OutputStream::try_default()
            .map_err(|e| format!("Sem dispositivo de áudio: {e}"))?;
        Ok(Engine {
            _stream: stream,
            handle,
            sink: None,
            sh: Arc::new(Shared::new()),
            volume: 0.8,
            base: 0.0,
            paused: false,
            path: None,
            duration: None,
            sample_rate: 0,
            channels: 0,
            kbps: None,
        })
    }

    fn open(path: &Path) -> Result<(Decoder<BufReader<File>>, u64), String> {
        let file = File::open(path).map_err(|e| format!("Não foi possível abrir o arquivo: {e}"))?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let dec = Decoder::new(BufReader::new(file))
            .map_err(|e| format!("Formato não suportado ou arquivo corrompido: {e}"))?;
        Ok((dec, size))
    }

    /// Carrega `path` e começa a tocar a partir de `start` segundos.
    pub fn load(&mut self, path: &Path, start: f64) -> Result<(), String> {
        self.stop();
        let (dec, size) = Self::open(path)?;
        let (rate, ch) = (dec.sample_rate(), dec.channels());

        if self.path.as_deref() != Some(path) {
            let mut dur = dec.total_duration().map(|d| d.as_secs_f64());
            if dur.is_none() && rate > 0 && ch > 0 {
                // MP3 sem cabeçalho de duração: conta as amostras uma vez.
                if let Ok((d2, _)) = Self::open(path) {
                    dur = Some(d2.count() as f64 / (rate as f64 * ch as f64));
                }
            }
            self.duration = dur;
            self.sample_rate = rate;
            self.channels = ch;
            self.kbps = dur
                .filter(|d| *d > 0.0)
                .map(|d| ((size as f64 * 8.0) / d / 1000.0).round() as u32);
            self.path = Some(path.to_path_buf());
        }

        let src = dec.skip_duration(Duration::from_secs_f64(start.max(0.0)));
        let sink = Sink::try_new(&self.handle).map_err(|e| format!("Falha ao criar saída de áudio: {e}"))?;
        sink.set_volume(self.volume);
        self.sh.frames.store(0, Relaxed);
        self.base = start.max(0.0);
        sink.append(Tap::new(src, self.sh.clone()));
        self.sink = Some(sink);
        self.paused = false;
        Ok(())
    }

    pub fn pause(&mut self) {
        if let Some(s) = &self.sink {
            s.pause();
            self.paused = true;
        }
    }

    pub fn resume(&mut self) {
        if let Some(s) = &self.sink {
            s.play();
            self.paused = false;
        }
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.sink.take() {
            s.stop();
        }
        self.base = 0.0;
        self.paused = false;
        self.sh.frames.store(0, Relaxed);
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        if let Some(s) = &self.sink {
            s.set_volume(self.volume);
        }
    }

    pub fn finished(&self) -> bool {
        !self.paused && self.sink.as_ref().map_or(false, |s| s.empty())
    }

    pub fn position(&self) -> f64 {
        if self.sink.is_none() || self.sample_rate == 0 {
            return 0.0;
        }
        self.base + self.sh.frames.load(Relaxed) as f64 / self.sample_rate as f64
    }

    /// Últimas `n` amostras (mono) capturadas.
    pub fn snapshot(&self, n: usize) -> Vec<f32> {
        match self.sh.scope.lock() {
            Ok(g) => g[g.len().saturating_sub(n)..].to_vec(),
            Err(_) => Vec::new(),
        }
    }
}

fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for i in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (i + k, i + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

/// Espectro em `bars` barras (escala logarítmica 50 Hz–16 kHz), valores em 0..1.
pub fn spectrum(samples: &[f32], bars: usize, sample_rate: f32) -> Vec<f32> {
    const N: usize = 1024;
    if samples.len() < 64 || bars == 0 || sample_rate <= 0.0 {
        return vec![0.0; bars];
    }
    let mut re = vec![0f32; N];
    let mut im = vec![0f32; N];
    let tail = &samples[samples.len().saturating_sub(N)..];
    for (i, s) in tail.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (N - 1) as f32).cos();
        re[i] = s * w;
    }
    fft(&mut re, &mut im);
    let bin_hz = sample_rate / N as f32;
    let edge = |b: usize| 50.0f32 * (16000.0f32 / 50.0).powf(b as f32 / bars as f32);
    (0..bars)
        .map(|b| {
            let lo = ((edge(b) / bin_hz) as usize).clamp(1, N / 2 - 1);
            let hi = ((edge(b + 1) / bin_hz) as usize).clamp(lo + 1, N / 2);
            let m = (lo..hi)
                .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt())
                .fold(0.0f32, f32::max);
            let db = 20.0 * (m / (N as f32 / 4.0) + 1e-9).log10();
            ((db + 70.0) / 70.0).clamp(0.0, 1.0)
        })
        .collect()
}