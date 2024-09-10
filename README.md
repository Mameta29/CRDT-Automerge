# Iroh Automerge プロジェクト

## 概要

このプロジェクトは、Iroh ネットワークライブラリを使用して、Automerge ベースの分散型共同編集システムを実装したものです。複数のノード間でリアルタイムにデータを同期し、一貫性を保ちながら共同作業を可能にする。

## 主な機能

- Iroh を使用した P2P ネットワーク通信
- Automerge を使用した分散型データ構造
- リアルタイムデータ同期
- コマンドラインインターフェース（CLI）での操作
- キーと値のペアによるドキュメント更新

## 必要条件

- Rust プログラミング言語
- Cargo（Rust のパッケージマネージャー）

## ローカルでの立ち上げ方法

1. リポジトリのクローン

   ```
   git clone https://github.com/Mameta29/CRDT-Automerge.git
   cd CRDT-Automerge
   ```

2. 依存関係をインストール

   ```
   cargo build
   ```

3. プログラム実行

   - 最初のノードを起動する場合：

     ```
     cargo run
     ```

   - 2 つ目以降のノードを起動し、既存のノードに接続する場合
     ```
     cargo run -- --remote-id [最初のノードのID]
     ```

   注意：最初のノードの ID は、プログラム起動時に表示される。

4. プログラムが起動したら、以下の形式でキーと値のペアを入力してドキュメントを更新できる

   ```
   key = value
   ```

5. 更新はリアルタイムで他のノードと同期される

6. プログラムを終了するには、Ctrl+C

## 今後の課題と計画

1. ユーザーインターフェースの改善

   - グラフィカルユーザーインターフェース（GUI）の実装
   - ウェブベースのインターフェースの開発

2. セキュリティの強化

   - エンドツーエンドの暗号化の実装
   - ユーザー認証システムの導入

3. 機能の拡張

   - 複雑なデータ構造のサポート（ネストされたオブジェクト、配列など）
   - バージョン管理と履歴表示機能の追加

4. テスト

   - 単体テストと統合テストの拡充

5. クロスプラットフォーム対応していきたい

   - モバイルアプリケーション（iOS/Android）
   - デスクトップアプリケーション（Windows/macOS/Linux）
