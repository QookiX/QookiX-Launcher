use fastnbt::Value;

const MAX_DEPTH: usize = 96;
const MAX_TAGS: usize = 524_288;
const MAX_ARRAY_ELEMENTS: usize = 16 * 1024 * 1024;

pub(super) fn decode(bytes: &[u8], label: &str) -> Result<Value, String> {
    let mut data = Input { remaining: bytes, tags: MAX_TAGS };
    let check = (|| {
        if data.byte()? != 10 {
            return Err("NBT 根节点必须是复合标签");
        }
        data.name()?;
        data.payload(10, 0)
    })();
    check.map_err(|err| format!("{label}: {err}"))?;
    fastnbt::from_bytes_with_opts(bytes, fastnbt::DeOpts::new().max_seq_len(MAX_ARRAY_ELEMENTS))
        .map_err(|err| format!("解析{label} NBT 失败: {err}"))
}

struct Input<'a> {
    remaining: &'a [u8],
    tags: usize,
}

impl<'a> Input<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], &'static str> {
        let (head, tail) = self.remaining.split_at_checked(len).ok_or("NBT 数据不完整")?;
        self.remaining = tail;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }

    fn name(&mut self) -> Result<(), &'static str> {
        let size = self.take(2)?;
        self.take(u16::from_be_bytes([size[0], size[1]]) as usize)?;
        Ok(())
    }

    fn len(&mut self) -> Result<usize, &'static str> {
        let bytes = self.take(4)?;
        let len = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        usize::try_from(len).ok().filter(|len| *len <= MAX_ARRAY_ELEMENTS)
            .ok_or("NBT 数组或列表长度超过限制")
    }

    fn payload(&mut self, tag: u8, depth: usize) -> Result<(), &'static str> {
        if depth > MAX_DEPTH {
            return Err("NBT 嵌套深度超过限制");
        }
        self.tags = self.tags.checked_sub(1).ok_or("NBT 标签数超过限制")?;
        match tag {
            1 => { self.take(1)?; }
            2 => { self.take(2)?; }
            3 | 5 => { self.take(4)?; }
            4 | 6 => { self.take(8)?; }
            8 => self.name()?,
            7 | 11 | 12 => {
                let width = match tag { 7 => 1, 11 => 4, _ => 8 };
                let size = self.len()?.checked_mul(width).ok_or("NBT 数组大小超过限制")?;
                self.take(size)?;
            }
            9 => {
                let item = self.byte()?;
                let len = self.len()?;
                if len > self.tags {
                    return Err("NBT 列表长度超过标签限制");
                }
                for _ in 0..len {
                    self.payload(item, depth + 1)?;
                }
            }
            10 => loop {
                let item = self.byte()?;
                if item == 0 {
                    break;
                }
                self.name()?;
                self.payload(item, depth + 1)?;
            },
            _ => return Err("NBT 标签类型无效"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_compounds_and_declared_array_sizes_have_limits() {
        let mut nested = vec![10, 0, 0];
        for _ in 0..=MAX_DEPTH {
            nested.extend_from_slice(&[10, 0, 1, b'a']);
        }
        nested.resize(nested.len() + MAX_DEPTH + 2, 0);
        assert!(decode(&nested, "NBT").unwrap_err().contains("嵌套深度"));
        let overflow = [10, 0, 0, 7, 0, 1, b'a', 127, 255, 255, 255, 0];
        assert!(decode(&overflow, "NBT").unwrap_err().contains("长度超过限制"));
    }
}
