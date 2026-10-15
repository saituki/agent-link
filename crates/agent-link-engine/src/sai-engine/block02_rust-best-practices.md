# 【ブロック2】Rustプログラミングのベストプラクティス：所有権、パターンマッチング、async/awaitについて解説する

## はじめに

Rust は、安全性とパフォーマンスを両立するシステムプログラミング言語です。
本ブロックでは、Rust プログラミングにおける三大重要概念である
**所有権（Ownership）**、**パターンマッチング（Pattern Matching）**、**非同期処理（async/await）** について、
ベストプラクティスを交えながら解説します。

---

## 1. 所有権（Ownership）

### 1.1 所有権とは

所有権は Rust の最も特徴的な機能であり、ガベージコレクタなしでメモリ安全を保証する仕組みです。
Rust では、すべての値はただ一つの**所有者**を持ちます。

#### 所有権の3つのルール

1. **Rust の各値は、所有者と呼ばれる変数を持つ**
2. **一度に存在できる所有者は1つだけ**
3. **所有者がスコープを抜けると、その値は破棄される**

```rust
fn main() {
    // s1 が文字列 "hello" の所有者になる
    let s1 = String::from("hello");

    // s1 の所有権が s2 にムーブされる
    let s2 = s1;

    // コンパイルエラー: s1 はすでに無効（ムーブ済み）
    // println!("{}", s1);

    println!("{}", s2); // OK
}
```

### 1.2 借用（Borrowing）

所有権を移動させずに値を参照する仕組みです。

```rust
fn main() {
    let s = String::from("hello");

    // 不変参照（読み取り専用）を渡す
    let len = calculate_length(&s);

    println!("'{}' の長さは {} です", s, len);
}

fn calculate_length(s: &String) -> usize {
    s.len()
}
```

#### 借用のルール

- **不変参照（`&T`）**: 同時に複数存在可能。読み取り専用。
- **可変参照（`&mut T`）**: 同時に1つだけ存在可能。書き込み可能。
- **不変参照と可変参照は共存できない**

```rust
let mut s = String::from("hello");

let r1 = &s;     // OK: 不変参照
let r2 = &s;     // OK: 不変参照（複数可）
// let r3 = &mut s; // エラー: 不変参照がある間は可変参照不可

println!("{} {}", r1, r2);
// r1, r2 はここで最後に使われる（Non-Lexical Lifetimes）

let r3 = &mut s; // OK: 不変参照が使われなくなった後
r3.push_str(" world");
```

### 1.3 ライフタイム（Lifetime）

ライフタイムは、参照が有効になるスコープを Rust が追跡する仕組みです。
多くの場合、コンパイラが自動的に推論します（ライフタイム省略規則）。

```rust
// 明示的なライフタイム指定が必要な例
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() {
        x
    } else {
        y
    }
}

fn main() {
    let s1 = String::from("abcdef");
    let s2 = String::from("xyz");

    let result = longest(&s1, &s2);
    println!("最も長い文字列: {}", result);
}
```

### 1.4 ベストプラクティス

| 原則 | 説明 |
|------|------|
| **可能な限り不変参照を使う** | 可変性は最小限に抑え、バグの温床を減らす |
| **大きなデータは借用で渡す** | ムーブよりも `&T` で不要なコピーを避ける |
| **Clone は必要なときだけ** | `clone()` はコストが高い。借用で済むなら借用を使う |
| **所有権のツリーを意識する** | 親がドロップされると子も自動的にドロップされる設計を活かす |

---

## 2. パターンマッチング（Pattern Matching）

### 2.1 パターンマッチングとは

Rust の `match` 式は、値の構造に基づいて網羅的な条件分岐を行う強力な制御構造です。
単なる `if-else` の代替ではなく、**値の分解（Destructuring）**と**網羅性チェック**を備えています。

### 2.2 基本構文

```rust
enum Coin {
    Penny,
    Nickel,
    Dime,
    Quarter(UsState), // 列挙型にデータを持たせられる
}

#[derive(Debug)]
enum UsState {
    Alabama,
    Alaska,
    // ...
}

fn value_in_cents(coin: Coin) -> u8 {
    match coin {
        Coin::Penny => 1,
        Coin::Nickel => 5,
        Coin::Dime => 10,
        Coin::Quarter(state) => {
            println!("{:?} 州のクォーター硬貨！", state);
            25
        }
    }
}
```

### 2.3 高度なパターン

```rust
fn main() {
    let x = Some(42);

    // ガード条件 (match guard)
    match x {
        Some(n) if n < 50 => println!("50未満: {}", n),
        Some(n) => println!("50以上: {}", n),
        None => println!("値なし"),
    }

    // 構造体の分解
    let point = Point { x: 10, y: 20 };
    match point {
        Point { x, y: 0 } => println!("x軸上: {}", x),
        Point { x: 0, y } => println!("y軸上: {}", y),
        Point { x, y } => println!("({}, {})", x, y),
    }

    // @ 束縛: パターン全体を変数に束縛
    match Some(3) {
        Some(n @ 1..=5) => println!("1から5の範囲の値 {} です", n),
        Some(n) => println!("範囲外: {}", n),
        None => println!("None"),
    }
}

struct Point {
    x: i32,
    y: i32,
}
```

### 2.4 if let / while let

```rust
// match の簡略形
let config_max = Some(3u8);
if let Some(max) = config_max {
    println!("最大値は {} に設定されています", max);
}

// ループ中にパターンマッチ
let mut stack = Vec::new();
stack.push(1);
stack.push(2);
stack.push(3);

while let Some(top) = stack.pop() {
    println!("{}", top);
}
// 出力: 3, 2, 1
```

### 2.5 Option / Result との組み合わせ

Rust のエラーハンドリングの基本は `Result<T, E>` と `match` です。

```rust
use std::fs::File;
use std::io::{self, Read};

fn read_file() -> Result<String, io::Error> {
    let mut file = File::open("hello.txt")?; // ?演算子: エラー時は早期リターン
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}

// match で明示的に処理する場合
fn read_file_verbose() -> String {
    let file = File::open("hello.txt");
    match file {
        Ok(mut f) => {
            let mut contents = String::new();
            match f.read_to_string(&mut contents) {
                Ok(_) => contents,
                Err(_) => String::from("読み取りエラー"),
            }
        }
        Err(_) => String::from("ファイルが見つかりません"),
    }
}
```

### 2.6 ベストプラクティス

| 原則 | 説明 |
|------|------|
| **網羅性を活かす** | `match` は全ケースを強制するため、列挙型にバリアントを追加した際のハンドリング漏れをコンパイル時に検出 |
| **ワイルドカード `_` は慎重に** | 将来のバリアント追加で意図しないハンドリングになる可能性がある |
| **`if let` は 1 ケースのみ関心がある場合に使う** | 残りのケースを無視してよいときは `if let` で簡潔に |
| **`?` 演算子を活用する** | `match` のネストを避け、エラーハンドリングを簡潔に記述 |
| **ガード条件で複雑さを分離** | 複雑な条件は `if` ガードで `match` アームの外に出して整理 |

---

## 3. 非同期処理（async/await）

### 3.1 async/await とは

Rust の async/await は、**ノンブロッキングな非同期プログラミング**を実現する構文です。
OS スレッドを使わず、軽量なタスク（Future）によって並行処理を実現します。

```rust
// Cargo.toml に以下を追加
// [dependencies]
// tokio = { version = "1", features = ["full"] }

use tokio;

#[tokio::main]
async fn main() {
    // 非同期関数の呼び出し
    let result = fetch_data().await;
    println!("取得結果: {}", result);
}

async fn fetch_data() -> String {
    // 非同期I/Oのシミュレーション
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    "データ取得成功".to_string()
}
```

### 3.2 Future と Poll

`async fn` はコンパイル時に `Future` トレイトを実装するステートマシンに変換されます。

```rust
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

// Future トレイトのシグネチャ（簡略化）
// trait Future {
//     type Output;
//     fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;
// }
```

### 3.3 並行実行

複数の Future を同時に実行するさまざまな方法があります。

```rust
use tokio;

#[tokio::main]
async fn main() {
    // 1. join!: すべての Future を並行実行、すべて完了するまで待つ
    let (res1, res2) = tokio::join!(
        task1(),
        task2(),
    );
    println!("join! 結果: {}, {}", res1, res2);

    // 2. select!: 最初に完了した Future の結果を取得
    tokio::select! {
        result = task1() => {
            println!("task1 が先に完了: {}", result);
        }
        result = task2() => {
            println!("task2 が先に完了: {}", result);
        }
    }

    // 3. spawn: 新しいタスクとしてバックグラウンド実行
    let handle = tokio::spawn(async {
        // バックグラウンド処理
        heavy_computation().await
    });

    // メインスレッドで別の処理...
    let main_result = task1().await;
    let bg_result = handle.await.unwrap();
    println!("メイン: {}, バックグラウンド: {}", main_result, bg_result);
}

async fn task1() -> String {
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    "タスク1完了".to_string()
}

async fn task2() -> String {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    "タスク2完了".to_string()
}

async fn heavy_computation() -> String {
    // CPUバウンドな処理は tokio::task::spawn_blocking を使う
    let result = tokio::task::spawn_blocking(|| {
        // 重い計算処理（スレッドプールで実行）
        (0..1_000_000).sum::<u64>()
    })
    .await
    .unwrap();
    format!("計算結果: {}", result)
}
```

### 3.4 ストリーム (Stream)

複数の非同期値のシーケンスを扱うには `Stream` トレイトを使います。

```rust
use tokio_stream::StreamExt;

// 要: cargo add tokio-stream

#[tokio::main]
async fn main() {
    // 非同期イテレーション
    let mut stream = tokio_stream::iter(0..5);

    while let Some(value) = stream.next().await {
        println!("ストリーム値: {}", value);
    }
}
```

### 3.5 Send + 'static 制約

`tokio::spawn` に渡す Future は `Send + 'static` でなければなりません。

```rust
// エラー例: 非 Send な型を spawn 内で使う
// let rc = std::rc::Rc::new(42);
// tokio::spawn(async move {
//     println!("{}", rc); // コンパイルエラー: Rc は Send でない
// });

// 解決: Arc を使う
let arc = std::sync::Arc::new(42);
let arc_clone = arc.clone();
tokio::spawn(async move {
    println!("{}", arc_clone); // OK
});
```

### 3.6 エラーハンドリング

```rust
use anyhow::{Result, anyhow};

async fn fetch_url(url: &str) -> Result<String> {
    let response = reqwest::get(url).await?;
    if !response.status().is_success() {
        return Err(anyhow!("HTTPエラー: {}", response.status()));
    }
    let body = response.text().await?;
    Ok(body)
}
```

### 3.7 ベストプラクティス

| 原則 | 説明 |
|------|------|
| **CPUバウンド処理は `spawn_blocking` を使う** | 非同期ランタイムのスレッドをブロックしない |
| **`Send` 制約を意識する** | `Rc` や `RefCell` の代わりに `Arc` や `Mutex`（非同期版は `tokio::sync::Mutex`）を使う |
| **キャンセル安全性を確保する** | `select!` でキャンセルされた Future が中途半端な状態を残さないように設計 |
| **デッドロックに注意** | 非同期 `Mutex` を `await` ポイントをまたいで保持しない |
| **過剰な `spawn` を避ける** | タスク数が増えすぎるとオーバーヘッド増加。`join!` や `FuturesUnordered` も検討 |
| **適切なランタイムを選ぶ** | Tokio（汎用）、async-std、smol など用途に応じて選択 |

---

## 4. まとめ

| 概念 | 本質 | 主なメリット |
|------|------|--------------|
| **所有権** | メモリ管理をコンパイル時に静的に保証 | ダングリングポインタ、二重解放、メモリリークを防止 |
| **パターンマッチング** | 値の構造を分解し網羅的に分岐 | ハンドリング漏れをコンパイル時に検出。コードの安全性と可読性を向上 |
| **async/await** | 軽量な非同期タスクによる並行処理 | OSスレッドより低コストで高スループットなI/O処理を実現 |

これら3つの概念を理解し適切に活用することで、Rust の真の力を引き出すことができます。
