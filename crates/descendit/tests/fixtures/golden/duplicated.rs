// Structurally duplicated function pairs, triggering the duplication dimension.

pub struct User {
    pub name: String,
    pub age: u32,
}

pub struct Product {
    pub title: String,
    pub price: f64,
}

pub struct Order {
    pub id: u64,
    pub quantity: u32,
}

// --- Pair 1: identical filtering logic ---

pub fn filter_users(items: &[User], min_age: u32) -> Vec<&User> {
    let mut result = Vec::new();
    for item in items {
        if item.age >= min_age {
            result.push(item);
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

pub fn filter_products(items: &[Product], min_price: f64) -> Vec<&Product> {
    let mut result = Vec::new();
    for item in items {
        if item.price >= min_price {
            result.push(item);
        }
    }
    result.sort_by(|a, b| a.title.cmp(&b.title));
    result
}

// --- Pair 2: identical counting logic ---

pub fn count_active_users(users: &[User]) -> usize {
    let mut count = 0;
    for user in users {
        if user.age > 0 {
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    count
}

pub fn count_available_products(products: &[Product]) -> usize {
    let mut count = 0;
    for product in products {
        if product.price > 0.0 {
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    count
}

// --- Pair 3: identical serialization pattern ---

pub fn serialize_users(users: &[User]) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("--- begin ---".to_string());
    for user in users {
        let line = format!("item: {}", user.name);
        lines.push(line);
    }
    lines.push("--- end ---".to_string());
    lines
}

pub fn serialize_products(products: &[Product]) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("--- begin ---".to_string());
    for product in products {
        let line = format!("item: {}", product.title);
        lines.push(line);
    }
    lines.push("--- end ---".to_string());
    lines
}

// --- Pair 4: identical validation pattern ---

pub fn validate_user(user: &User) -> Result<(), String> {
    if user.name.is_empty() {
        return Err("name is empty".to_string());
    }
    if user.name.len() > 100 {
        return Err("name too long".to_string());
    }
    if user.age > 150 {
        return Err("age out of range".to_string());
    }
    Ok(())
}

pub fn validate_order(order: &Order) -> Result<(), String> {
    if order.id == 0 {
        return Err("id is empty".to_string());
    }
    if order.id > 999_999_999 {
        return Err("id too long".to_string());
    }
    if order.quantity > 10_000 {
        return Err("quantity out of range".to_string());
    }
    Ok(())
}
