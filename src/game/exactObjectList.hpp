#ifndef LIERO_EXACTOBJECTLIST_HPP
#define LIERO_EXACTOBJECTLIST_HPP

#include <cstddef>
#include <cassert>
#include <vector>
#include <cstring>
#include <algorithm>
#include <gvl/support/bits.hpp>

struct ExactObjectListBase
{
	bool used;
};

template<typename T>
struct ExactObjectList
{
	struct range
	{
		range(T* cur, T* end)
		: cur(cur), end(end)
		{
		}

		T* next()
		{
			while (!cur->used)
				++cur;

			T* ret = cur;
			++cur;

			return ret == end ? 0 : ret;
		}

		T* cur;
		T* end;
	};

	ExactObjectList()
	: limit_(0), count_(0)
	{
	}

	void init(int limit)
	{
		limit_ = limit;
		arr_.resize(limit + 1); // +1 for sentinel
		freeList_.resize((limit + 31) / 32);
		clear();
	}

	T* getFreeObject()
	{
		assert(count_ < (std::size_t)limit_);
		++count_;

		T* ptr = 0;
		for (int i = 0; i < (int)freeList_.size(); ++i)
		{
			if (freeList_[i] != 0)
			{
				int bit = gvl_bottom_bit(freeList_[i]);
				uint32_t index = ((uint32_t)i << 5) + (uint32_t)bit;
				ptr = arr_.data() + index;
				freeList_[i] &= ~(uint32_t(1) << bit);
				break;
			}
		}

		assert(ptr && !ptr->used && ptr >= arr_.data() && ptr < arr_.data() + limit_);
		ptr->used = true;

		return ptr;
	}

	T* newObjectReuse()
	{
		T* ret;
		if (count_ >= (std::size_t)limit_)
			ret = &arr_[limit_ - 1];
		else
			ret = getFreeObject();

		assert(ret->used && ret >= arr_.data() && ret < arr_.data() + limit_);
		return ret;
	}

	T* newObject()
	{
		if (count_ >= (std::size_t)limit_)
			return 0;

		T* ret = getFreeObject();
		assert(ret->used && ret >= arr_.data() && ret < arr_.data() + limit_);
		return ret;
	}

	range all()
	{
		return range(arr_.data(), arr_.data() + limit_);
	}

	void free(T* ptr)
	{
		assert(ptr->used);
		if (ptr->used)
		{
			uint32_t index = uint32_t(ptr - arr_.data());
			freeList_[index >> 5] |= (uint32_t(1) << (index & 31));

			ptr->used = false;

			assert(count_ > 0);
			--count_;
		}
	}

	void free(range& r)
	{
		free(r.cur - 1);
	}

	void clear()
	{
		if (limit_ == 0) return;

		std::fill(freeList_.begin(), freeList_.end(), 0xFFFFFFFFu);
		count_ = 0;

		for (int i = 0; i < limit_; ++i)
			arr_[i].used = false;

		// Sentinel: always "used" so range iteration terminates
		arr_[limit_].used = true;

		// Mark padding bits (beyond limit) as unavailable
		int freeListSize = (int)freeList_.size();
		for (uint32_t index = (uint32_t)limit_; index < (uint32_t)(freeListSize * 32); ++index)
			freeList_[index >> 5] &= ~(uint32_t(1) << (index & 31));
	}

	std::size_t size() const
	{
		return count_;
	}

	// Direct pointer to the underlying array (for index arithmetic)
	T* data() { return arr_.data(); }
	const T* data() const { return arr_.data(); }

private:
	int limit_;
	std::vector<T> arr_;
	std::vector<uint32_t> freeList_;
	std::size_t count_;
};

#endif // LIERO_EXACTOBJECTLIST_HPP
