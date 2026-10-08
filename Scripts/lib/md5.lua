--[[
    纯 Lua 的 MD5（只依赖 LuaJIT 自带的 `bit` 库）。

    为什么要它：小牛翻译那类接口的签名（authStr）是「把参数按名字 ASCII 升序拼成 k=v&k=v…
    再取 MD5 的小写十六进制」，而 Lua 标准库没有 MD5。放这里给脚本 `dofile` 用。

    用法（脚本里第一行就能拿到它）：

        local md5 = dofile("Scripts/lib/md5.lua").hex     -- 装机目录下（Server 的工作目录就是安装目录）
        local auth = md5("apikey=xxx&appId=yyy&from=zh&…")

    注意：这份文件在 `Scripts\lib\` 子目录里 —— 加载器**只认脚本目录根部**的 `*.lua`，
    子目录里的东西不会被当成脚本执行，所以放工具函数的东西是安全的。
]]

local bit = bit or require("bit")

local band, bor, bxor, bnot, rol = bit.band, bit.bor, bit.bxor, bit.bnot, bit.rol

-- 每一轮左移的位数（RFC 1321）
local S = {
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
}

-- 常量表（RFC 1321；写成十六进制字面量，避免用 sin() 现算带来的浮点边角）
local K = {
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee,
    0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
    0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
    0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
    0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
    0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
    0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
}

-- 32 位小端的四个字节（`word` 可能是负数：先取模转成无符号）——填充里的长度用它
local function bytes(word)
    word = word % 4294967296
    return string.char(
        word % 256,
        math.floor(word / 256) % 256,
        math.floor(word / 65536) % 256,
        math.floor(word / 16777216) % 256
    )
end

-- 32 位小端的八个十六进制字符——最后的结果用它
local function hex_word(word)
    word = word % 4294967296
    return string.format(
        "%02x%02x%02x%02x",
        word % 256,
        math.floor(word / 256) % 256,
        math.floor(word / 65536) % 256,
        math.floor(word / 16777216) % 256
    )
end

-- 小端读一个 32 位字
local function word_at(message, index)
    local b1, b2, b3, b4 = string.byte(message, index, index + 3)
    return b1 + b2 * 256 + b3 * 65536 + b4 * 16777216
end

-- 算 `message` 的 MD5，返回 32 位小写十六进制。
local function hex(message)
    message = message or ""
    local bits = #message * 8
    -- 填充：先补 0x80，再补零到 56 (mod 64)，最后 8 字节放小端的 bit 长度
    message = message .. "\128"
    while #message % 64 ~= 56 do
        message = message .. "\0"
    end
    local low = bits % 4294967296
    local high = math.floor(bits / 4294967296)
    message = message .. bytes(low) .. bytes(high)

    local a0, b0, c0, d0 = 0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476
    for chunk = 0, #message - 1, 64 do
        local x = {}
        for i = 0, 15 do
            x[i] = word_at(message, chunk + i * 4 + 1)
        end
        local a, b, c, d = a0, b0, c0, d0
        for i = 0, 63 do
            local f, g
            if i < 16 then
                f = bor(band(b, c), band(bnot(b), d))
                g = i
            elseif i < 32 then
                f = bor(band(d, b), band(bnot(d), c))
                g = (5 * i + 1) % 16
            elseif i < 48 then
                f = bxor(b, bxor(c, d))
                g = (3 * i + 5) % 16
            else
                f = bxor(c, bor(b, bnot(d)))
                g = (7 * i) % 16
            end
            -- 每一步都按 32 位取模，别让 double 的精度掺进来
            local sum = band(a + f + x[g] + K[i + 1], 0xffffffff)
            local next_b = band(b + rol(sum, S[i + 1]), 0xffffffff)
            a, b, c, d = d, next_b, b, c
        end
        a0 = band(a0 + a, 0xffffffff)
        b0 = band(b0 + b, 0xffffffff)
        c0 = band(c0 + c, 0xffffffff)
        d0 = band(d0 + d, 0xffffffff)
    end
    return hex_word(a0) .. hex_word(b0) .. hex_word(c0) .. hex_word(d0)
end

return { hex = hex }
