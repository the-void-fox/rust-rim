use std::collections::HashSet;

use super::{ModDb, ModId};

/// Активная сборка: какие моды включены и в каком порядке они грузятся.
///
/// Порядок элементов `order` — и есть порядок загрузки; это то, что уезжает
/// в `<activeMods>` ModsConfig.xml. Раньше порядок был неявно закодирован
/// позициями внутри общего `Vec<ModEntry>`, из-за чего любое перемещение
/// требовало пересчёта индексов активных модов.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    order: Vec<ModId>,
    active: HashSet<ModId>,
}

impl Profile {
    pub fn new() -> Self {
        Self::default()
    }

    /// Собирает сборку из списка строк (ModsConfig.xml или файл-список).
    ///
    /// Строки разрешаются через каталог, поэтому работают и packageId, и
    /// числовые Workshop ID. Неизвестные и повторяющиеся записи отбрасываются
    /// — включить можно только то, что реально установлено.
    pub fn from_raw_ids<I, S>(raw: I, db: &ModDb) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut profile = Self::new();
        for item in raw {
            match db.resolve(item.as_ref()) {
                Some(id) => profile.activate(id),
                None => tracing::debug!("Unknown mod in list: {}", item.as_ref()),
            }
        }
        profile
    }

    pub fn is_active(&self, id: &ModId) -> bool {
        self.active.contains(id)
    }

    /// Идентификаторы в порядке загрузки.
    pub fn order(&self) -> &[ModId] {
        &self.order
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn position(&self, id: &ModId) -> Option<usize> {
        if !self.active.contains(id) {
            return None;
        }
        self.order.iter().position(|x| x == id)
    }

    /// Включает мод в конец списка загрузки. Повторный вызов — не операция.
    pub fn activate(&mut self, id: ModId) {
        if self.active.insert(id.clone()) {
            self.order.push(id);
        }
    }

    /// Включает мод на конкретную позицию (drag & drop из неактивного списка).
    pub fn activate_at(&mut self, id: ModId, pos: usize) {
        if self.active.insert(id.clone()) {
            self.order.insert(pos.min(self.order.len()), id);
        } else {
            self.move_to(&id, pos);
        }
    }

    pub fn deactivate(&mut self, id: &ModId) {
        if self.active.remove(id) {
            self.order.retain(|x| x != id);
        }
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.active.clear();
    }

    /// Оставляет только моды, для которых предикат вернул `true`.
    pub fn retain(&mut self, keep: impl Fn(&ModId) -> bool) {
        self.order.retain(|id| keep(id));
        self.active.retain(|id| keep(id));
    }

    /// Перемещает мод на позицию `pos` в порядке загрузки.
    pub fn move_to(&mut self, id: &ModId, pos: usize) {
        let Some(from) = self.position(id) else { return };
        let to = pos.min(self.order.len() - 1);
        if from == to {
            return;
        }
        let entry = self.order.remove(from);
        self.order.insert(to, entry);
    }

    pub fn move_up(&mut self, id: &ModId) {
        if let Some(pos) = self.position(id) {
            if pos > 0 {
                self.order.swap(pos, pos - 1);
            }
        }
    }

    pub fn move_down(&mut self, id: &ModId) {
        if let Some(pos) = self.position(id) {
            if pos + 1 < self.order.len() {
                self.order.swap(pos, pos + 1);
            }
        }
    }

    /// Заменяет порядок целиком (результат сортировки).
    /// Состав сборки при этом обязан сохраниться.
    pub fn set_order(&mut self, order: Vec<ModId>) {
        debug_assert_eq!(
            order.len(),
            self.order.len(),
            "порядок сборки не должен менять её состав",
        );
        self.order = order;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> ModId {
        ModId::new(s)
    }

    fn profile(ids: &[&str]) -> Profile {
        let mut p = Profile::new();
        for i in ids {
            p.activate(id(i));
        }
        p
    }

    #[test]
    fn activate_is_idempotent() {
        let mut p = profile(&["a", "b"]);
        p.activate(id("a"));
        assert_eq!(p.order(), &[id("a"), id("b")]);
    }

    #[test]
    fn deactivate_removes_from_order() {
        let mut p = profile(&["a", "b", "c"]);
        p.deactivate(&id("b"));
        assert_eq!(p.order(), &[id("a"), id("c")]);
        assert!(!p.is_active(&id("b")));
    }

    #[test]
    fn move_to_reorders_without_changing_membership() {
        let mut p = profile(&["a", "b", "c"]);
        p.move_to(&id("c"), 0);
        assert_eq!(p.order(), &[id("c"), id("a"), id("b")]);

        p.move_to(&id("c"), 99); // за пределы — прижимается к концу
        assert_eq!(p.order(), &[id("a"), id("b"), id("c")]);
    }

    #[test]
    fn move_up_down_at_edges_is_noop() {
        let mut p = profile(&["a", "b"]);
        p.move_up(&id("a"));
        p.move_down(&id("b"));
        assert_eq!(p.order(), &[id("a"), id("b")]);
    }

    #[test]
    fn moving_inactive_mod_does_nothing() {
        let mut p = profile(&["a"]);
        p.move_to(&id("ghost"), 0);
        p.move_up(&id("ghost"));
        assert_eq!(p.order(), &[id("a")]);
    }

    #[test]
    fn activate_at_inserts_at_position() {
        let mut p = profile(&["a", "b"]);
        p.activate_at(id("c"), 1);
        assert_eq!(p.order(), &[id("a"), id("c"), id("b")]);
    }

    #[test]
    fn activate_at_on_active_mod_moves_it() {
        let mut p = profile(&["a", "b", "c"]);
        p.activate_at(id("c"), 0);
        assert_eq!(p.order(), &[id("c"), id("a"), id("b")]);
        assert_eq!(p.len(), 3);
    }
}
