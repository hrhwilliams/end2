export type UserId = string;

export type DirectoryIndex = {
  index: string;
  proof: string;
};

export type UserInfo = {
  id: string;
  username: string;
  nickname?: string;
  // Absent on channel participants; only the user endpoints return it.
  directory?: DirectoryIndex;
};
