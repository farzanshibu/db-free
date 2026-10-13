// SOT: demo-database, sample-ecommerce-sqlite, demo-seed-sql

use crate::error::{AppError, AppResult};
use rusqlite::Connection;
use std::path::Path;

// ============================================================================
// WHAT:  The sample database behind "Try the demo": a small e-commerce schema
//        (categories, products, customers, orders, order items, reviews, plus two
//        views) filled with a few thousand deterministic rows.
// WHY:   A first run with nothing to connect to should still show a grid, a
//        foreign key, a diagram and a query worth running. Generated with SQL at
//        run time rather than shipped as a binary resource, so there is nothing
//        to keep in sync and the data is reviewable in this file.
// HOW:   Pure SQL: word lists in temp tables and recursive counters, with
//        arithmetic instead of `random()` so two builds are identical (tests rely
//        on the counts). Foreign keys are declared, so the ER diagram and the
//        FK cell picker have something to show.
// WHERE: src-tauri/src/services/connection_files.rs (create_demo)
// ============================================================================

pub const DEMO_FILE_NAME: &str = "db-free-demo.sqlite";
pub const DEMO_CUSTOMERS: i64 = 200;
pub const DEMO_PRODUCTS: i64 = 60;
pub const DEMO_ORDERS: i64 = 600;

const DEMO_SQL: &str = "
PRAGMA foreign_keys = ON;

CREATE TABLE categories (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT
);

CREATE TABLE products (
    id INTEGER PRIMARY KEY,
    category_id INTEGER NOT NULL REFERENCES categories (id),
    sku TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    price REAL NOT NULL CHECK (price >= 0),
    stock INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    first_name TEXT NOT NULL,
    last_name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    country TEXT NOT NULL,
    city TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE orders (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers (id),
    status TEXT NOT NULL CHECK (status IN ('pending', 'paid', 'shipped', 'delivered', 'cancelled')),
    ordered_at TEXT NOT NULL,
    shipped_at TEXT,
    total REAL NOT NULL DEFAULT 0
);

CREATE TABLE order_items (
    id INTEGER PRIMARY KEY,
    order_id INTEGER NOT NULL REFERENCES orders (id),
    product_id INTEGER NOT NULL REFERENCES products (id),
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    unit_price REAL NOT NULL
);

CREATE TABLE reviews (
    id INTEGER PRIMARY KEY,
    product_id INTEGER NOT NULL REFERENCES products (id),
    customer_id INTEGER NOT NULL REFERENCES customers (id),
    rating INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    title TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX products_category_idx ON products (category_id);
CREATE INDEX orders_customer_idx ON orders (customer_id);
CREATE INDEX orders_status_idx ON orders (status);
CREATE INDEX order_items_order_idx ON order_items (order_id);
CREATE INDEX order_items_product_idx ON order_items (product_id);
CREATE INDEX reviews_product_idx ON reviews (product_id);

CREATE TEMP TABLE seq (n INTEGER PRIMARY KEY);
WITH RECURSIVE counter(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM counter WHERE n < 600)
INSERT INTO seq SELECT n FROM counter;

CREATE TEMP TABLE words (kind TEXT NOT NULL, idx INTEGER NOT NULL, word TEXT NOT NULL, PRIMARY KEY (kind, idx));
INSERT INTO words (kind, idx, word) VALUES
    ('adjective', 0, 'Classic'), ('adjective', 1, 'Compact'), ('adjective', 2, 'Deluxe'), ('adjective', 3, 'Eco'),
    ('adjective', 4, 'Pro'), ('adjective', 5, 'Slim'), ('adjective', 6, 'Smart'), ('adjective', 7, 'Travel'),
    ('adjective', 8, 'Ultra'), ('adjective', 9, 'Urban'),
    ('noun', 0, 'Backpack'), ('noun', 1, 'Bottle'), ('noun', 2, 'Desk Lamp'), ('noun', 3, 'Headphones'),
    ('noun', 4, 'Keyboard'), ('noun', 5, 'Mug'), ('noun', 6, 'Notebook'), ('noun', 7, 'Speaker'),
    ('noun', 8, 'Stand'), ('noun', 9, 'Tote'), ('noun', 10, 'Watch'), ('noun', 11, 'Wallet'),
    ('first', 0, 'Ada'), ('first', 1, 'Ben'), ('first', 2, 'Chloe'), ('first', 3, 'Dev'), ('first', 4, 'Elena'),
    ('first', 5, 'Farah'), ('first', 6, 'Grace'), ('first', 7, 'Hugo'), ('first', 8, 'Iris'), ('first', 9, 'Jon'),
    ('first', 10, 'Kira'), ('first', 11, 'Liam'), ('first', 12, 'Mia'), ('first', 13, 'Noah'), ('first', 14, 'Omar'),
    ('last', 0, 'Almeida'), ('last', 1, 'Brown'), ('last', 2, 'Chen'), ('last', 3, 'Diaz'), ('last', 4, 'Evans'),
    ('last', 5, 'Fischer'), ('last', 6, 'Garcia'), ('last', 7, 'Haddad'), ('last', 8, 'Ito'), ('last', 9, 'Jones'),
    ('last', 10, 'Khan'), ('last', 11, 'Lopez'), ('last', 12, 'Moreau'), ('last', 13, 'Novak'),
    ('review', 0, 'Exactly what I needed'), ('review', 1, 'Solid build'), ('review', 2, 'Does the job'),
    ('review', 3, 'Not worth the price'), ('review', 4, 'Would buy again'), ('review', 5, 'Arrived late but great');

CREATE TEMP TABLE places (idx INTEGER PRIMARY KEY, country TEXT NOT NULL, city TEXT NOT NULL);
INSERT INTO places (idx, country, city) VALUES
    (0, 'United States', 'Austin'), (1, 'United States', 'Boston'), (2, 'Canada', 'Toronto'),
    (3, 'United Kingdom', 'Leeds'), (4, 'Germany', 'Hamburg'), (5, 'France', 'Lyon'),
    (6, 'India', 'Pune'), (7, 'Japan', 'Osaka'), (8, 'Brazil', 'Recife'), (9, 'Australia', 'Perth');

INSERT INTO categories (id, name, description) VALUES
    (1, 'Bags', 'Backpacks, totes and travel gear'),
    (2, 'Drinkware', 'Bottles, mugs and flasks'),
    (3, 'Lighting', 'Desk and floor lamps'),
    (4, 'Audio', 'Headphones and speakers'),
    (5, 'Computing', 'Keyboards, stands and accessories'),
    (6, 'Stationery', 'Notebooks and writing'),
    (7, 'Wearables', 'Watches and trackers'),
    (8, 'Accessories', 'Wallets and small goods');

INSERT INTO products (id, category_id, sku, name, price, stock, active, created_at)
SELECT s.n,
       1 + (s.n % 8),
       printf('SKU-%04d', s.n),
       (SELECT word FROM words WHERE kind = 'adjective' AND idx = s.n % 10) || ' ' ||
       (SELECT word FROM words WHERE kind = 'noun' AND idx = s.n % 12),
       ROUND(9 + ((s.n * 37) % 190) + ((s.n * 13) % 100) / 100.0, 2),
       (s.n * 17) % 120,
       CASE WHEN s.n % 11 = 0 THEN 0 ELSE 1 END,
       date('2024-01-01', '+' || ((s.n * 5) % 300) || ' days')
FROM seq s WHERE s.n <= 60;

INSERT INTO customers (id, first_name, last_name, email, country, city, created_at)
SELECT s.n,
       (SELECT word FROM words WHERE kind = 'first' AND idx = s.n % 15),
       (SELECT word FROM words WHERE kind = 'last' AND idx = (s.n * 7) % 14),
       lower((SELECT word FROM words WHERE kind = 'first' AND idx = s.n % 15)) || '.' ||
       lower((SELECT word FROM words WHERE kind = 'last' AND idx = (s.n * 7) % 14)) || s.n || '@example.com',
       (SELECT country FROM places WHERE idx = (s.n * 3) % 10),
       (SELECT city FROM places WHERE idx = (s.n * 3) % 10),
       date('2024-01-01', '+' || ((s.n * 3) % 330) || ' days')
FROM seq s WHERE s.n <= 200;

INSERT INTO orders (id, customer_id, status, ordered_at, shipped_at, total)
SELECT s.n,
       1 + ((s.n * 37) % 200),
       CASE s.n % 10 WHEN 0 THEN 'pending' WHEN 1 THEN 'cancelled' WHEN 2 THEN 'paid' WHEN 3 THEN 'shipped'
                     ELSE 'delivered' END,
       date('2025-01-01', '+' || ((s.n * 11) % 365) || ' days'),
       CASE WHEN s.n % 10 IN (3, 4, 5, 6, 7, 8, 9)
            THEN date('2025-01-01', '+' || (((s.n * 11) % 365) + 2) || ' days') END,
       0
FROM seq s WHERE s.n <= 600;

INSERT INTO order_items (order_id, product_id, quantity, unit_price)
SELECT o.id,
       1 + ((o.id * 13 + k.n * 7) % 60),
       1 + ((o.id + k.n) % 4),
       p.price
FROM orders o
JOIN seq k ON k.n <= 1 + (o.id % 3)
JOIN products p ON p.id = 1 + ((o.id * 13 + k.n * 7) % 60);

UPDATE orders SET total = COALESCE((SELECT ROUND(SUM(quantity * unit_price), 2) FROM order_items WHERE order_id = orders.id), 0);

INSERT INTO reviews (product_id, customer_id, rating, title, created_at)
SELECT 1 + ((s.n * 7) % 60),
       1 + ((s.n * 29) % 200),
       1 + ((s.n * 3) % 5),
       (SELECT word FROM words WHERE kind = 'review' AND idx = s.n % 6),
       date('2025-02-01', '+' || ((s.n * 2) % 300) || ' days')
FROM seq s WHERE s.n <= 250;

CREATE VIEW order_summary AS
SELECT o.id AS order_id,
       c.first_name || ' ' || c.last_name AS customer,
       c.country,
       o.status,
       o.ordered_at,
       COUNT(i.id) AS line_items,
       o.total
FROM orders o
JOIN customers c ON c.id = o.customer_id
LEFT JOIN order_items i ON i.order_id = o.id
GROUP BY o.id;

CREATE VIEW product_ratings AS
SELECT p.id AS product_id, p.name, cat.name AS category,
       COUNT(r.id) AS reviews, ROUND(AVG(r.rating), 2) AS average_rating
FROM products p
JOIN categories cat ON cat.id = p.category_id
LEFT JOIN reviews r ON r.product_id = p.id
GROUP BY p.id;

DROP TABLE seq;
DROP TABLE words;
DROP TABLE places;
";

// WHAT:  Builds the sample schema and data into an open connection.
pub fn populate(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(DEMO_SQL).map_err(AppError::store)
}

// WHAT:  Writes a fresh sample database at `path`, replacing a half-written one.
// HOW:   Built in a sibling temp file and renamed into place, so a crash while
//        generating never leaves a truncated database that later opens as empty.
pub fn build_demo_database(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::store)?;
    }
    let staging = path.with_extension("sqlite.tmp");
    let _ = std::fs::remove_file(&staging);
    {
        let conn = Connection::open(&staging).map_err(AppError::store)?;
        populate(&conn)?;
        conn.close().map_err(|(_, e)| AppError::store(e))?;
    }
    std::fs::rename(&staging, path).map_err(AppError::store)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap_or(-1)
    }

    #[test]
    fn the_demo_has_the_documented_shape_and_is_deterministic() {
        let a = Connection::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        populate(&a).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(count(&a, "categories"), 8);
        assert_eq!(count(&a, "products"), DEMO_PRODUCTS);
        assert_eq!(count(&a, "customers"), DEMO_CUSTOMERS);
        assert_eq!(count(&a, "orders"), DEMO_ORDERS);
        assert_eq!(count(&a, "reviews"), 250);
        assert!(count(&a, "order_items") > DEMO_ORDERS, "every order has at least one line");
        assert!(count(&a, "order_summary") > 0 && count(&a, "product_ratings") == DEMO_PRODUCTS);
        // Temp helper tables are gone; only the real schema remains.
        let leftovers: i64 = a.query_row("SELECT COUNT(*) FROM sqlite_temp_master", [], |r| r.get(0)).unwrap_or(-1);
        assert_eq!(leftovers, 0);

        let b = Connection::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        populate(&b).unwrap_or_else(|e| panic!("{e}"));
        let fingerprint = |c: &Connection| -> String {
            c.query_row("SELECT group_concat(total) FROM (SELECT total FROM orders ORDER BY id LIMIT 50)", [], |r| r.get(0)).unwrap_or_default()
        };
        assert_eq!(fingerprint(&a), fingerprint(&b), "two builds are identical");
    }

    #[test]
    fn relationships_hold_and_totals_match_their_lines() {
        let conn = Connection::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        populate(&conn).unwrap_or_else(|e| panic!("{e}"));
        let violations: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap_or(-1);
        assert_eq!(violations, 0, "no dangling foreign keys");
        let mismatched: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM orders o WHERE ABS(o.total - (SELECT SUM(quantity * unit_price) FROM order_items WHERE order_id = o.id)) > 0.01",
                [],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        assert_eq!(mismatched, 0);
        let emails: i64 = conn.query_row("SELECT COUNT(DISTINCT email) FROM customers", [], |r| r.get(0)).unwrap_or(-1);
        assert_eq!(emails, DEMO_CUSTOMERS);
    }

    #[test]
    fn building_to_a_path_replaces_a_partial_file() {
        let dir = std::env::temp_dir().join(format!("db-free-demo-{}", uuid::Uuid::new_v4()));
        let path = dir.join(DEMO_FILE_NAME);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(path.with_extension("sqlite.tmp"), b"garbage").unwrap_or_else(|e| panic!("{e}"));
        build_demo_database(&path).unwrap_or_else(|e| panic!("{e}"));
        let conn = Connection::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(count(&conn, "customers"), DEMO_CUSTOMERS);
        assert!(!path.with_extension("sqlite.tmp").exists());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
