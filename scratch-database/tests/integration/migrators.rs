//! Two migrators a test hands in, each making a table of its own, so a database shows which one made it.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;

// One migration creating the table it is named for.
struct Table(&'static str);

impl MigrationName for Table {
  fn name(&self) -> &str {
    self.0
  }
}

#[async_trait]
impl MigrationTrait for Table {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .get_connection()
      .execute_unprepared(&format!("CREATE TABLE {} (id integer PRIMARY KEY)", self.0))
      .await
      .map(|_| ())
  }
}

/// The migrator making the table `kept`.
pub struct Kept;

impl MigratorTrait for Kept {
  fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(Table("kept"))]
  }
}

/// The migrator making the table `other`.
pub struct Other;

impl MigratorTrait for Other {
  fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(Table("other"))]
  }
}
