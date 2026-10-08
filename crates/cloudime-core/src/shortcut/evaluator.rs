//! 算式求值：复数内核（实部 + 虚部）。
//!
//! - 运算符：`+ - * /`、`%`（求余）、`^`（乘方，右结合）、`!`（阶乘，只对非负整数，`5!`）、
//!   `@`（复数模，**只能后缀**：`(5+6i)@`；前缀 `@x` 是语法错）。
//! - 函数：`sin/cos/tan/cot`（角度）、`sinr/cosr/tanr/cotr`（弧度）、`asin/acos/atan`（返回角度）、
//!   `lg/ln/log(底数,真数)`、`sqrt`（收复数，`sqrt(-4)` = `2i`）、`sinh/cosh/tanh`、
//!   `arrange(样本数,总体数)`、`combine(样本数,总体数)`、
//!   `avg/vari/sum(逗号隔开的数组)`（`vari` 是总体方差，除以 n；`sum` 是总和）。
//! - **不做隐式乘法**：`5*i`、`5*sin(30)` 里的 `*` 都要写出来；`6i`、`2(3+4)`、`2x3` 都是语法错
//!   （`x` 也不再当乘号）。虚数单位就是标识符 `i`。
//! - 结果：整数不带小数点，小数最多十位去尾零；虚部不为零写 `a+bi`（`i` / `-i` 不写 1）。
//! - 算不了（语法错、除零、函数参数不对、溢出 / 非有限）一律返回 `None` —— 调用方那边就是「空候选」。

/// 复数。全部运算在它上面做，虚部恰好为 0 就按实数输出。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Complex {
    /// 实部。
    re: f64,

    /// 虚部。
    im: f64,
}

impl Complex {
    const ZERO: Self = Self { re: 0.0, im: 0.0 };
    const ONE: Self = Self { re: 1.0, im: 0.0 };
    const I: Self = Self { re: 0.0, im: 1.0 };

    /// 建一个复数：顺手把 `-0.0` 收成 `0.0` —— 一元负号会给出 `-0.0`，它在 `atan2` 里算成
    /// 「负方向」，`sqrt(-4)` 就会得到 `-2i` 这种不合理的主值。
    fn new(re: f64, im: f64) -> Self {
        Self {
            re: re + 0.0,
            im: im + 0.0,
        }
    }

    fn real(re: f64) -> Self {
        Self::new(re, 0.0)
    }

    /// 实部虚部都有限（溢出与 NaN 都算「算不了」）。
    fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }

    /// 虚部恰为 0（实数）。
    fn is_real(self) -> bool {
        self.im == 0.0
    }

    fn add(self, other: Self) -> Self {
        Self::new(self.re + other.re, self.im + other.im)
    }

    fn sub(self, other: Self) -> Self {
        Self::new(self.re - other.re, self.im - other.im)
    }

    fn mul(self, other: Self) -> Self {
        Self::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }

    /// 除法；除数为零返回 `None`。
    fn div(self, other: Self) -> Option<Self> {
        let divisor = other.re * other.re + other.im * other.im;
        if divisor == 0.0 {
            return None;
        }
        Some(Self::new(
            (self.re * other.re + self.im * other.im) / divisor,
            (self.im * other.re - self.re * other.im) / divisor,
        ))
    }

    /// 复数的模 `|z|`。
    fn modulus(self) -> f64 {
        (self.re * self.re + self.im * self.im).sqrt()
    }

    /// 乘方：两边都是实数时走 `powf`（`2^10` 精确是 1024），否则用极坐标 `a^b = e^(b·ln a)`
    /// （负数的非整数次方在实数里是 NaN，会落到这条路，`(-1)^0.5` = `i`）。
    fn pow(self, other: Self) -> Option<Self> {
        if self.is_real() && other.is_real() {
            let value = self.re.powf(other.re);
            if value.is_finite() {
                return Some(Self::real(value));
            }
        }
        if self.re == 0.0 && self.im == 0.0 {
            return match (other.re, other.im) {
                (0.0, 0.0) => Some(Self::ONE),
                (re, 0.0) if re > 0.0 => Some(Self::ZERO),
                _ => None,
            };
        }
        let logarithm = Self::new(self.modulus().ln(), self.im.atan2(self.re));
        other.mul(logarithm).exp()
    }

    /// `e^z`。
    fn exp(self) -> Option<Self> {
        let scale = self.re.exp();
        Some(Self::new(scale * self.im.cos(), scale * self.im.sin()).denoise())
    }

    /// 把极坐标那条路带出来的浮点噪声收掉：`sqrt(-4)` 的实部是 `1.2e-16` 这种「数值上的零」，
    /// 不收掉会显示成一长串。只按**相对**量级判断，不动真正的小数。
    fn denoise(self) -> Self {
        let scale = self.re.abs().max(self.im.abs());
        if scale == 0.0 {
            return self;
        }
        let tolerance = 1e-12 * scale;
        Self::new(
            if self.re.abs() < tolerance {
                0.0
            } else {
                self.re
            },
            if self.im.abs() < tolerance {
                0.0
            } else {
                self.im
            },
        )
    }
}

/// 算式求值。算不了返回 `None`，算出来是最终显示文本。
pub fn evaluate(text: &str) -> Option<String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        position: 0,
    };
    let value = parser.expression()?;
    if parser.position != parser.bytes.len() || !value.is_finite() {
        return None;
    }
    Some(format_complex(value.denoise()))
}

/// 递归下降求值器，只在 [`evaluate`] 里用。
struct Parser<'a> {
    /// 算式文本。
    bytes: &'a [u8],

    /// 当前读到的位置。
    position: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    /// 吃掉一个字节；不是它就什么都不做。
    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    /// 跳过空格（面板里能敲空格，算式本身不吃它，统一跳过更省心）。
    fn skip_spaces(&mut self) {
        while self.peek() == Some(b' ') {
            self.position += 1;
        }
    }

    /// 加减。
    fn expression(&mut self) -> Option<Complex> {
        self.skip_spaces();
        let mut value = self.term()?;
        loop {
            self.skip_spaces();
            let op = match self.peek() {
                Some(op @ (b'+' | b'-')) => op,
                _ => break,
            };
            self.position += 1;
            let rhs = self.term()?;
            value = if op == b'+' {
                value.add(rhs)
            } else {
                value.sub(rhs)
            };
        }
        Some(value)
    }

    /// 乘、除、求余。`x` 不是乘号。
    fn term(&mut self) -> Option<Complex> {
        self.skip_spaces();
        let mut value = self.unary()?;
        loop {
            self.skip_spaces();
            let op = match self.peek() {
                Some(op @ (b'*' | b'/' | b'%')) => op,
                _ => break,
            };
            self.position += 1;
            let rhs = self.unary()?;
            value = match op {
                b'*' => value.mul(rhs),
                b'/' => value.div(rhs)?,
                // 求余只对实数（而且除数不为零）有意义
                _ => {
                    if !value.is_real() || !rhs.is_real() || rhs.re == 0.0 {
                        return None;
                    }
                    Complex::real(value.re % rhs.re)
                }
            };
        }
        Some(value)
    }

    /// 一元正负号，往下是乘方。
    fn unary(&mut self) -> Option<Complex> {
        self.skip_spaces();
        match self.peek() {
            Some(b'-') => {
                self.position += 1;
                self.unary().map(|value| Complex::new(-value.re, -value.im))
            }
            Some(b'+') => {
                self.position += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// 乘方（右结合）：`2^3^2` = 512，`2^-3` 也行。优先级低于后缀、高于一元负号。
    fn power(&mut self) -> Option<Complex> {
        let base = self.postfix()?;
        self.skip_spaces();
        if self.eat(b'^') {
            let exponent = self.unary()?;
            return base.pow(exponent);
        }
        Some(base)
    }

    /// 后缀：`!` 阶乘、`@` 复数模；可以连写（`3!@`）。
    fn postfix(&mut self) -> Option<Complex> {
        let mut value = self.atom()?;
        loop {
            self.skip_spaces();
            if self.eat(b'!') {
                value = Complex::real(factorial(value)?);
            } else if self.eat(b'@') {
                value = Complex::real(value.modulus());
            } else {
                return Some(value);
            }
        }
    }

    /// 数字、虚数单位 `i`、函数调用、括号。
    fn atom(&mut self) -> Option<Complex> {
        self.skip_spaces();
        if self.eat(b'(') {
            let value = self.expression()?;
            self.skip_spaces();
            if !self.eat(b')') {
                return None;
            }
            return Some(value);
        }
        if matches!(self.peek(), Some(b'0'..=b'9' | b'.')) {
            return self.number();
        }
        let start = self.position;
        while matches!(self.peek(), Some(byte) if byte.is_ascii_alphabetic()) {
            self.position += 1;
        }
        if start == self.position {
            return None;
        }
        let name = std::str::from_utf8(&self.bytes[start..self.position])
            .ok()?
            .to_ascii_lowercase();
        self.skip_spaces();
        if self.peek() != Some(b'(') {
            // 不是函数调用：只有虚数单位 `i` 是合法的标识符
            return (name == "i").then_some(Complex::I);
        }
        self.position += 1;
        let mut args = Vec::new();
        loop {
            args.push(self.expression()?);
            self.skip_spaces();
            if self.eat(b',') {
                continue;
            }
            break;
        }
        self.skip_spaces();
        if !self.eat(b')') {
            return None;
        }
        call(&name, &args)
    }

    /// 十进制数字（带小数点）。
    fn number(&mut self) -> Option<Complex> {
        let start = self.position;
        while matches!(self.peek(), Some(b'0'..=b'9' | b'.')) {
            self.position += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.position])
            .ok()?
            .parse::<f64>()
            .ok()
            .map(Complex::real)
    }
}

/// 阶乘：只对非负整数（实数）有意义。170 以上必然溢出，直接给个无穷，交给「非有限」那道判断。
fn factorial(value: Complex) -> Option<f64> {
    if !value.is_real() || value.re < 0.0 || value.re.fract() != 0.0 {
        return None;
    }
    if value.re > 170.0 {
        return Some(f64::INFINITY);
    }
    let mut result = 1.0;
    let mut step = 2.0;
    while step <= value.re {
        result *= step;
        step += 1.0;
    }
    Some(result)
}

/// 排列数 `P(n, k) = n × (n-1) × … × (n-k+1)`（一个一个乘，避免先算两个巨大阶乘再相除）。
fn permutations(n: u64, k: u64) -> f64 {
    (0..k).fold(1.0, |acc, index| acc * (n - index) as f64)
}

/// 组合数 `C(n, k)`：乘一项除一项，尽量落在能整除的位置（`k` 取小的那一半）。
fn combinations(n: u64, k: u64) -> f64 {
    let k = k.min(n - k);
    (0..k).fold(1.0, |acc, index| {
        acc * (n - index) as f64 / (index + 1) as f64
    })
}

/// 两个参数都是非负整数才认（排列组合用）。
fn two_integers(args: &[Complex]) -> Option<(u64, u64)> {
    let [first, second] = args else {
        return None;
    };
    let integer = |value: &Complex| -> Option<u64> {
        (value.is_real() && value.re >= 0.0 && value.re.fract() == 0.0 && value.re <= 170.0)
            .then_some(value.re as u64)
    };
    Some((integer(first)?, integer(second)?))
}

/// 只收实数的数组（统计函数用）；空数组或有一个不是实数都返回 `None`。
fn real_args(args: &[Complex]) -> Option<Vec<f64>> {
    if args.is_empty() {
        return None;
    }
    args.iter()
        .map(|value| value.is_real().then_some(value.re))
        .collect()
}

/// 只收一个实数、只返回一个实数的函数（三角函数、对数、双曲都走它）。
/// 定义域外的结果会是 NaN，上面「非有限」那道判断会把它变成「算不了」。
fn real_call(args: &[Complex], function: impl Fn(f64) -> f64) -> Option<Complex> {
    let [value] = args else {
        return None;
    };
    if !value.is_real() {
        return None;
    }
    Some(Complex::real(function(value.re)))
}

/// 函数调用：名字 + 实参。名字不认识、参数个数不对、参数类型不对一律 `None`。
fn call(name: &str, args: &[Complex]) -> Option<Complex> {
    /// 角度 → 弧度。
    const DEGREE: f64 = std::f64::consts::PI / 180.0;

    match name {
        // (1) 三角函数：带 r 的收弧度，不带的收角度。切函数在 90° 这种「除零」点上不是无穷大
        // 而是 1.6e16 这种大数，这里当错误处理（NaN → 算不了）。
        "sin" => real_call(args, |x| (x * DEGREE).sin()),
        "cos" => real_call(args, |x| (x * DEGREE).cos()),
        "tan" => real_call(args, |x| {
            let radians = x * DEGREE;
            if radians.cos().abs() < 1e-12 {
                f64::NAN
            } else {
                radians.tan()
            }
        }),
        "cot" => real_call(args, |x| {
            let radians = x * DEGREE;
            if radians.sin().abs() < 1e-12 {
                f64::NAN
            } else {
                1.0 / radians.tan()
            }
        }),
        "sinr" => real_call(args, f64::sin),
        "cosr" => real_call(args, f64::cos),
        "tanr" => real_call(args, |x| {
            if x.cos().abs() < 1e-12 {
                f64::NAN
            } else {
                x.tan()
            }
        }),
        "cotr" => real_call(args, |x| {
            if x.sin().abs() < 1e-12 {
                f64::NAN
            } else {
                1.0 / x.tan()
            }
        }),
        // (3) 反三角：返回角度（与 sin/cos 收角度配成一对）
        "asin" => real_call(args, |x| x.asin() / DEGREE),
        "acos" => real_call(args, |x| x.acos() / DEGREE),
        "atan" => real_call(args, |x| x.atan() / DEGREE),
        // (4) 双曲
        "sinh" => real_call(args, f64::sinh),
        "cosh" => real_call(args, f64::cosh),
        "tanh" => real_call(args, f64::tanh),
        // (2) 对数与开方
        "lg" => real_call(args, f64::log10),
        "ln" => real_call(args, f64::ln),
        "log" => {
            let [base, value] = args else {
                return None;
            };
            if !base.is_real() || !value.is_real() {
                return None;
            }
            Some(Complex::real(value.re.log(base.re)))
        }
        // sqrt 收复数：负数开方给虚数（`sqrt(-4)` = `2i`）
        "sqrt" => {
            let [value] = args else {
                return None;
            };
            value.pow(Complex::real(0.5))
        }
        // (5) 排列组合：第一个参数是样本数、第二个是总体数
        "arrange" => {
            let (sample, total) = two_integers(args)?;
            if sample > total {
                return None;
            }
            Some(Complex::real(permutations(total, sample)))
        }
        "combine" => {
            let (sample, total) = two_integers(args)?;
            if sample > total {
                return None;
            }
            Some(Complex::real(combinations(total, sample)))
        }
        // (6) 统计：逗号隔开的数组（vari 是总体方差）
        "sum" => Some(Complex::real(real_args(args)?.iter().sum())),
        "avg" => {
            let values = real_args(args)?;
            Some(Complex::real(
                values.iter().sum::<f64>() / values.len() as f64,
            ))
        }
        "vari" => {
            let values = real_args(args)?;
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let variance = values
                .iter()
                .map(|value| (value - mean) * (value - mean))
                .sum::<f64>()
                / values.len() as f64;
            Some(Complex::real(variance))
        }
        _ => None,
    }
}

/// 结果转成显示文本：实数沿用原来那套（整数不带小数点、小数最多十位去尾零），
/// 虚部不为零时写 `a+bi`（`i` / `-i` 不写 1）。
fn format_complex(value: Complex) -> String {
    if value.im == 0.0 {
        return format_number(value.re);
    }
    if value.re == 0.0 {
        return if value.im < 0.0 {
            format!("-{}", format_imaginary(-value.im))
        } else {
            format_imaginary(value.im)
        };
    }
    let sign = if value.im < 0.0 { '-' } else { '+' };
    format!(
        "{}{sign}{}",
        format_number(value.re),
        format_imaginary(value.im.abs())
    )
}

/// 正的虚部：`i` / `2i` / `1.5i`。
fn format_imaginary(im: f64) -> String {
    if im == 1.0 {
        "i".to_owned()
    } else {
        format!("{}i", format_number(im))
    }
}

/// 整数不带小数点，小数最多十位、去掉末尾的零。
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{value:.0}");
    }
    let text = format!("{value:.10}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        assert_eq!(evaluate("1+2").as_deref(), Some("3"));
        assert_eq!(evaluate("2*3+4").as_deref(), Some("10"));
        assert_eq!(evaluate("2*(3+4)").as_deref(), Some("14"));
        assert_eq!(evaluate("1/3").as_deref(), Some("0.3333333333"));
        assert_eq!(evaluate("2^10").as_deref(), Some("1024"));
        assert_eq!(evaluate("2^3^2").as_deref(), Some("512"));
        assert_eq!(evaluate("2^-2").as_deref(), Some("0.25"));
        assert_eq!(evaluate("-3+5").as_deref(), Some("2"));
        assert_eq!(evaluate("0.1+0.2").as_deref(), Some("0.3"));
        assert_eq!(evaluate("1.5*2").as_deref(), Some("3"));
        assert_eq!(evaluate("7%3").as_deref(), Some("1"));
        assert_eq!(evaluate("5!").as_deref(), Some("120"));
        assert_eq!(evaluate("3!^2").as_deref(), Some("36"));
        assert_eq!(evaluate("2^3!").as_deref(), Some("64"));
    }

    /// `x` 不再当乘号，也没有隐式乘法：`*` 得写出来。
    #[test]
    fn multiplication_must_be_explicit() {
        assert_eq!(evaluate("2x3"), None);
        assert_eq!(evaluate("2(3+4)"), None);
        assert_eq!(evaluate("6i"), None);
        assert_eq!(evaluate("2*3").as_deref(), Some("6"));
        assert_eq!(evaluate("2*(3+4)").as_deref(), Some("14"));
    }

    /// 复数：`i` 是虚数单位，乘要写 `*`；模用后缀 `@`，前缀是语法错。
    #[test]
    fn complex_numbers() {
        assert_eq!(evaluate("i").as_deref(), Some("i"));
        assert_eq!(evaluate("-i").as_deref(), Some("-i"));
        assert_eq!(evaluate("i^2").as_deref(), Some("-1"));
        assert_eq!(evaluate("(1+2*i)*(3-i)").as_deref(), Some("5+5i"));
        assert_eq!(evaluate("3+4*i").as_deref(), Some("3+4i"));
        assert_eq!(evaluate("1/i").as_deref(), Some("-i"));
        assert_eq!(evaluate("sqrt(-4)").as_deref(), Some("2i"));
        assert_eq!(evaluate("sqrt(9)").as_deref(), Some("3"));
        assert_eq!(evaluate("(5+6*i)@").as_deref(), Some("7.8102496759"));
        assert_eq!(evaluate("@(5+6*i)"), None);
        assert_eq!(evaluate("(-1)^0.5").as_deref(), Some("i"));
    }

    /// 函数那一套：三角函数（角度 / 弧度）、反三角、对数、双曲、排列组合、统计。
    #[test]
    fn functions() {
        assert_eq!(evaluate("sin(30)").as_deref(), Some("0.5"));
        assert_eq!(evaluate("cos(60)").as_deref(), Some("0.5"));
        assert_eq!(evaluate("cot(45)").as_deref(), Some("1"));
        assert_eq!(evaluate("sinr(0)").as_deref(), Some("0"));
        assert_eq!(evaluate("cosr(0)").as_deref(), Some("1"));
        assert_eq!(evaluate("asin(0.5)").as_deref(), Some("30"));
        assert_eq!(evaluate("atan(1)").as_deref(), Some("45"));
        assert_eq!(evaluate("lg(1000)").as_deref(), Some("3"));
        assert_eq!(evaluate("ln(1)").as_deref(), Some("0"));
        assert_eq!(evaluate("log(2,8)").as_deref(), Some("3"));
        assert_eq!(evaluate("sinh(0)").as_deref(), Some("0"));
        assert_eq!(evaluate("cosh(0)").as_deref(), Some("1"));
        assert_eq!(evaluate("tanh(0)").as_deref(), Some("0"));
        assert_eq!(evaluate("arrange(2,5)").as_deref(), Some("20"));
        assert_eq!(evaluate("combine(2,5)").as_deref(), Some("10"));
        assert_eq!(evaluate("combine(5,5)").as_deref(), Some("1"));
        assert_eq!(evaluate("sum(1,2,3)").as_deref(), Some("6"));
        assert_eq!(evaluate("avg(1,2,3)").as_deref(), Some("2"));
        assert_eq!(evaluate("vari(1,2,3)").as_deref(), Some("0.6666666667"));
        // 带运算符的参数
        assert_eq!(evaluate("sum(1,2*3)").as_deref(), Some("7"));
        // 名字不认识 / 参数个数不对 / 参数类型不对
        assert_eq!(evaluate("nope(1)"), None);
        assert_eq!(evaluate("sin(1,2)"), None);
        assert_eq!(evaluate("log(2)"), None);
        assert_eq!(evaluate("sin(i)"), None);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(evaluate("1+"), None);
        assert_eq!(evaluate("1/0"), None);
        assert_eq!(evaluate("(1+2"), None);
        assert_eq!(evaluate("abc"), None);
        assert_eq!(evaluate(""), None);
        assert_eq!(evaluate("1e400"), None);
        assert_eq!(evaluate("9^9^9"), None);
        // 阶乘只收非负整数；余数不能除零；定义域外的函数
        assert_eq!(evaluate("(-1)!"), None);
        assert_eq!(evaluate("2.5!"), None);
        assert_eq!(evaluate("i!"), None);
        assert_eq!(evaluate("1%0"), None);
        assert_eq!(evaluate("ln(0)"), None);
        assert_eq!(evaluate("asin(2)"), None);
        assert_eq!(evaluate("tan(90)"), None);
        assert_eq!(evaluate("cot(0)"), None);
        // 排列组合的第一个参数不能比第二个大
        assert_eq!(evaluate("arrange(3,2)"), None);
    }
}
