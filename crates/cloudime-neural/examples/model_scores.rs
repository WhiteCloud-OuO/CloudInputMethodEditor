//! 给 `tools/model-export/parity.py` 生成 candle 参考分。
//!
//! 用法：`cargo run --release -p cloudime-neural --example model_scores -- <模型.qjm> <输出.tsv>`。
//! 输出格式与脚本约定：`CONTEXT` / `SCORE` / `END`，UTF-8、制表符分隔。

use std::path::Path;

use cloudime_neural::CharScorer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    let _ = args.next();
    let Some(path) = args.next() else {
        eprintln!("用法: model_scores <模型.qjm> [输出.tsv]");
        std::process::exit(2);
    };
    let out = args.next();
    let scorer = CharScorer::load(Path::new(&path))?;
    let cases: &[(&str, &[&str])] = &[
        ("", &["你好", "云朵输入法", "今天天气不错"]),
        ("今天", &["天气", "天气不错", "天"]),
        (
            "今天天气不错我们一起去公园散步吧",
            &[
                "今天天气不错",
                "今天天气很好",
                "明天天气不错",
                "我们去公园",
                "去公园散步",
                "去公园散步吧",
                "我们一起去公园散步",
                "今天天气不错我们一起去公园散步吧",
            ],
        ),
        (
            "这是一个很长的前文，用来测试上下文截断是否与训练脚本一致。云朵输入法是一个本地输入法，它的候选窗口由服务进程绘制，模型推理也在本地完成。我们继续写一些字，让前文超过模型的一百二十八个 token 上限，看看截断以后分数是否还对得上。",
            &[
                "云朵输入法",
                "本地输入法",
                "服务进程",
                "模型推理",
                "继续写一些字",
            ],
        ),
    ];
    let mut output = String::new();
    for (context, candidates) in cases {
        let scores = scorer.score(context, candidates)?;
        output.push_str(&format!("CONTEXT\t{context}\n"));
        for (candidate, score) in candidates.iter().zip(scores) {
            output.push_str(&format!("SCORE\t{candidate}\t{score:.10}\n"));
        }
        output.push_str("END\n");
    }
    if let Some(path) = out {
        std::fs::write(path, &output)?;
    } else {
        print!("{output}");
    }
    Ok(())
}
