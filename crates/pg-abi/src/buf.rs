//! Little-endian primitives that the input and output encodings are built from.

use egui::{Color32, Pos2, Rect, Vec2};

use crate::DecodeError;

/// Appends little-endian values to a byte buffer.
pub struct Writer<'a> {
    out: &'a mut Vec<u8>,
}

impl<'a> Writer<'a> {
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        Self { out }
    }

    pub fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }

    pub fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn usize(&mut self, v: usize) {
        self.u32(v as u32);
    }

    pub fn f32(&mut self, v: f32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f64(&mut self, v: f64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn bytes(&mut self, v: &[u8]) {
        self.usize(v.len());
        self.out.extend_from_slice(v);
    }

    pub fn raw(&mut self, v: &[u8]) {
        self.out.extend_from_slice(v);
    }

    pub fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }

    pub fn pos2(&mut self, v: Pos2) {
        self.f32(v.x);
        self.f32(v.y);
    }

    pub fn vec2(&mut self, v: Vec2) {
        self.f32(v.x);
        self.f32(v.y);
    }

    pub fn rect(&mut self, v: Rect) {
        self.pos2(v.min);
        self.pos2(v.max);
    }

    pub fn color32(&mut self, v: Color32) {
        self.raw(&v.to_array());
    }

    /// Writes a tag byte of 0 for `None`, or 1 followed by the payload.
    pub fn option<T>(&mut self, v: Option<T>, write: impl FnOnce(&mut Self, T)) {
        match v {
            None => self.u8(0),
            Some(v) => {
                self.u8(1);
                write(self, v);
            }
        }
    }
}

/// Reads little-endian values from a byte slice.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub fn raw(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(len).ok_or(DecodeError::UnexpectedEnd)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(DecodeError::UnexpectedEnd)?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let slice = self.raw(N)?;
        let mut buf = [0; N];
        buf.copy_from_slice(slice);
        Ok(buf)
    }

    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }

    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.u8()? != 0)
    }

    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn usize(&mut self) -> Result<usize, DecodeError> {
        Ok(self.u32()? as usize)
    }

    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(self.array()?))
    }

    pub fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.usize()?;
        self.raw(len)
    }

    pub fn str(&mut self) -> Result<&'a str, DecodeError> {
        core::str::from_utf8(self.bytes()?).map_err(|_| DecodeError::InvalidUtf8)
    }

    pub fn string(&mut self) -> Result<String, DecodeError> {
        Ok(self.str()?.to_owned())
    }

    pub fn pos2(&mut self) -> Result<Pos2, DecodeError> {
        Ok(Pos2::new(self.f32()?, self.f32()?))
    }

    pub fn vec2(&mut self) -> Result<Vec2, DecodeError> {
        Ok(Vec2::new(self.f32()?, self.f32()?))
    }

    pub fn rect(&mut self) -> Result<Rect, DecodeError> {
        Ok(Rect::from_min_max(self.pos2()?, self.pos2()?))
    }

    pub fn color32(&mut self) -> Result<Color32, DecodeError> {
        let [r, g, b, a] = self.array::<4>()?;
        Ok(Color32::from_rgba_premultiplied(r, g, b, a))
    }

    pub fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Option<T>, DecodeError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(read(self)?)),
            tag => Err(DecodeError::BadTag {
                what: "Option",
                tag: tag as u32,
            }),
        }
    }
}
